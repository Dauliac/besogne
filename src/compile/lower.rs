use crate::ir::types::*;
use crate::manifest::{self, Input, Phase, Flag, FlagKind};
use std::collections::{HashMap, HashSet};

/// Lower a parsed manifest into the intermediate representation
pub fn lower_manifest(manifest: &manifest::Manifest) -> Result<BesogneIR, String> {
    // Version is the content hash of the manifest — deterministic, no user-specified version
    let manifest_json = serde_json::to_vec(manifest)
        .map_err(|e| format!("cannot serialize manifest for hashing: {e}"))?;
    let version = blake3::hash(&manifest_json).to_hex()[..16].to_string();

    let metadata = Metadata {
        name: manifest.name.clone(),
        version,
        description: manifest.description.clone(),
    };

    let sandbox = resolve_sandbox(&manifest.sandbox);

    let besogne_name_upper = manifest.name.to_uppercase().replace('-', "_");
    let flags = lower_and_validate_flags(&manifest.flags, &besogne_name_upper)?;

    let mut resolved_inputs = Vec::new();

    // Generate synthetic env inputs for each flag
    for flag in &flags {
        let env_input = ResolvedInput {
            id: ContentId::from_content("env", &flag.env_var, flag.env_var.as_bytes()),
            phase: Phase::Pre,
            input: ResolvedNativeInput::Env {
                name: flag.env_var.clone(),
                value: flag.default.as_ref().and_then(|d| {
                    d.as_str().map(|s| s.to_string()).or_else(|| {
                        d.as_bool().map(|b| if b { "1".to_string() } else { "0".to_string() })
                    })
                }),
                expect: None,
                secret: false,
            },
            after: vec![],
            from_plugin: Some("flag".to_string()),
            sealed: None,
        };
        resolved_inputs.push(env_input);
    }

    for (key, input) in &manifest.inputs {
        match input {
            Input::Plugin(p) => {
                return Err(format!(
                    "input '{key}': plugin '{}' not expanded before lowering (bug in compile pipeline)",
                    p.plugin
                ));
            }
            _ => {
                let resolved = lower_input(key, input)?;
                resolved_inputs.push(resolved);
            }
        }
    }

    // Resolve exec-phase ordering constraints
    resolve_ordering(&mut resolved_inputs, &manifest.inputs)?;

    Ok(BesogneIR {
        metadata,
        sandbox,
        flags,
        inputs: resolved_inputs,
        verify_first_run: manifest.verify_first_run,
    })
}

/// Lower a manifest input to IR. The `key` is the map key (= the input's name).
fn lower_input(key: &str, input: &Input) -> Result<ResolvedInput, String> {
    let (native, phase, id) = match input {
        Input::Env(e) => {
            let env_name = e.name.clone().unwrap_or_else(|| key.to_string());
            let native = ResolvedNativeInput::Env {
                name: env_name.clone(),
                value: e.value.clone(),
                expect: e.expect.clone(),
                secret: e.secret.unwrap_or(false),
            };
            let phase = e.phase.clone().unwrap_or(Phase::Pre);
            let id = ContentId::from_content("env", &env_name, env_name.as_bytes());
            (native, phase, id)
        }

        Input::File(f) => {
            let native = ResolvedNativeInput::File {
                path: f.path.clone(),
                expect: f.expect.clone(),
                permissions: f.permissions.clone(),
            };
            let phase = f.phase.clone().unwrap_or(Phase::Pre);
            let id = ContentId::from_content("file", &f.path, f.path.as_bytes());
            (native, phase, id)
        }

        Input::Binary(b) => {
            let bin_name = b.name.clone().unwrap_or_else(|| key.to_string());
            let version_constraint = b
                .version
                .clone()
                .or_else(|| extract_version_constraint(&b.validate));
            let native = ResolvedNativeInput::Binary {
                name: bin_name.clone(),
                path: b.path.clone(),
                version_constraint,
                source: None,
                resolved_path: None,
                resolved_version: None,
                binary_hash: None,
            };
            let phase = b.phase.clone().unwrap_or(Phase::Build);
            let id = ContentId::from_content("binary", &bin_name, bin_name.as_bytes());
            (native, phase, id)
        }

        Input::Service(s) => {
            let identifier = s.tcp.as_deref()
                .or(s.http.as_deref())
                .unwrap_or(key);
            let native = ResolvedNativeInput::Service {
                name: Some(key.to_string()),
                tcp: s.tcp.clone(),
                http: s.http.clone(),
            };
            let phase = s.phase.clone().unwrap_or(Phase::Pre);
            let id = ContentId::from_content("service", identifier, identifier.as_bytes());
            (native, phase, id)
        }

        Input::Command(c) => {
            let run_resolved = resolve_run_spec(&c.run);
            let native = ResolvedNativeInput::Command {
                name: key.to_string(),
                run: run_resolved,
                env: c.env.clone().unwrap_or_default(),
                ensure: c.ensure.clone().unwrap_or_default(),
                side_effects: c.side_effects.unwrap_or(false),
                output: c.output.clone(),
            };
            let phase = c.phase.clone().unwrap_or(Phase::Exec);
            let id = ContentId::from_content("command", key, key.as_bytes());
            (native, phase, id)
        }

        Input::User(u) => {
            let identifier = u.in_group.as_deref().unwrap_or("current");
            let native = ResolvedNativeInput::User {
                in_group: u.in_group.clone(),
            };
            let phase = u.phase.clone().unwrap_or(Phase::Pre);
            let id = ContentId::from_content("user", identifier, identifier.as_bytes());
            (native, phase, id)
        }

        Input::Platform(p) => {
            let identifier = format!(
                "{}-{}",
                p.os.as_deref().unwrap_or("any"),
                p.arch.as_deref().unwrap_or("any")
            );
            let native = ResolvedNativeInput::Platform {
                os: p.os.clone(),
                arch: p.arch.clone(),
            };
            let phase = p.phase.clone().unwrap_or(Phase::Build);
            let id = ContentId::from_content("platform", &identifier, identifier.as_bytes());
            (native, phase, id)
        }

        Input::Dns(d) => {
            let native = ResolvedNativeInput::Dns {
                host: d.host.clone(),
                expect: d.expect.clone(),
            };
            let phase = d.phase.clone().unwrap_or(Phase::Pre);
            let id = ContentId::from_content("dns", &d.host, d.host.as_bytes());
            (native, phase, id)
        }

        Input::Metric(m) => {
            let native = ResolvedNativeInput::Metric {
                metric: m.metric.clone(),
                path: m.path.clone(),
            };
            let phase = m.phase.clone().unwrap_or(Phase::Pre);
            let id = ContentId::from_content("metric", &m.metric, m.metric.as_bytes());
            (native, phase, id)
        }

        Input::Plugin(_) => {
            return Err("plugins should be expanded before lowering".into());
        }
    };

    Ok(ResolvedInput {
        id,
        phase,
        input: native,
        after: vec![],
        from_plugin: None,
        sealed: None,
    })
}

/// Resolve the run spec (was: exec) into a flat command vec
fn resolve_run_spec(spec: &manifest::ExecSpec) -> Vec<String> {
    match spec {
        manifest::ExecSpec::Array(args) => args.clone(),
        manifest::ExecSpec::Shell(s) => vec!["sh".into(), "-c".into(), s.clone()],
        manifest::ExecSpec::Pipe { pipe } => {
            let parts: Vec<String> = pipe
                .iter()
                .map(|cmd| cmd.join(" "))
                .collect();
            vec!["sh".into(), "-c".into(), parts.join(" | ")]
        }
        manifest::ExecSpec::Script { file, args } => {
            let mut cmd = vec![file.clone()];
            if let Some(a) = args {
                cmd.extend(a.clone());
            }
            cmd
        }
    }
}

/// Resolve `after:` ordering constraints for exec-phase inputs
fn resolve_ordering(
    inputs: &mut Vec<ResolvedInput>,
    manifest_inputs: &HashMap<String, manifest::Input>,
) -> Result<(), String> {
    // Build name→id map for exec-phase inputs
    let name_to_id: HashMap<String, ContentId> = inputs
        .iter()
        .filter(|i| i.phase == Phase::Exec)
        .filter_map(|i| {
            if let ResolvedNativeInput::Command { name, .. } = &i.input {
                Some((name.clone(), i.id.clone()))
            } else if let ResolvedNativeInput::Service { name: Some(name), .. } = &i.input {
                Some((name.clone(), i.id.clone()))
            } else {
                None
            }
        })
        .collect();

    // Collect `after` constraints from manifest (key = input name)
    let mut after_by_name: HashMap<String, Vec<String>> = HashMap::new();
    for (key, mi) in manifest_inputs {
        match mi {
            manifest::Input::Command(c) => {
                if let Some(after) = &c.after {
                    after_by_name.insert(key.clone(), after.clone());
                }
            }
            manifest::Input::Service(s) => {
                if let Some(after) = &s.after {
                    after_by_name.insert(key.clone(), after.clone());
                }
            }
            _ => {}
        }
    }

    // Resolve string refs → ContentIds
    let resolutions: Vec<(usize, Vec<ContentId>)> = inputs
        .iter()
        .enumerate()
        .filter(|(_, i)| i.phase == Phase::Exec)
        .filter_map(|(idx, input)| {
            let cmd_name = match &input.input {
                ResolvedNativeInput::Command { name, .. } => Some(name),
                ResolvedNativeInput::Service { name: Some(name), .. } => Some(name),
                _ => None,
            }?;

            let after_names = after_by_name.get(cmd_name)?;
            let resolved: Result<Vec<ContentId>, String> = after_names
                .iter()
                .map(|dep_name| {
                    name_to_id.get(dep_name).cloned().ok_or_else(|| {
                        format!(
                            "command '{cmd_name}' has after: ['{dep_name}'] which is not an exec-phase input"
                        )
                    })
                })
                .collect();

            match resolved {
                Ok(ids) => Some(Ok((idx, ids))),
                Err(e) => Some(Err(e)),
            }
        })
        .collect::<Result<Vec<_>, _>>()?;

    for (idx, after_ids) in resolutions {
        inputs[idx].after = after_ids;
    }

    Ok(())
}

fn extract_version_constraint(
    validate: &Option<HashMap<String, serde_json::Value>>,
) -> Option<String> {
    validate.as_ref().and_then(|v| {
        v.get("version").and_then(|ver| {
            ver.get("range").and_then(|r| r.as_str().map(|s| s.to_string()))
        })
    })
}

fn compute_flag_env_var(flag: &Flag, besogne_name_upper: &str) -> String {
    if let Some(env) = &flag.env {
        return env.clone();
    }
    let flag_upper = flag.name.to_uppercase().replace('-', "_");
    if let Some(sub) = &flag.subcommand {
        let sub_upper = sub.to_uppercase().replace('-', "_");
        format!("{besogne_name_upper}_{sub_upper}_{flag_upper}")
    } else {
        format!("{besogne_name_upper}_{flag_upper}")
    }
}

fn derive_short(name: &str, taken: &HashSet<char>) -> Option<char> {
    if let Some(c) = name.chars().next() {
        let c = c.to_ascii_lowercase();
        if c.is_ascii_alphabetic() && !taken.contains(&c) {
            return Some(c);
        }
        let cu = c.to_ascii_uppercase();
        if !taken.contains(&cu) {
            return Some(cu);
        }
    }
    for segment in name.split('-').skip(1) {
        if let Some(c) = segment.chars().next() {
            let c = c.to_ascii_lowercase();
            if c.is_ascii_alphabetic() && !taken.contains(&c) {
                return Some(c);
            }
            let cu = c.to_ascii_uppercase();
            if !taken.contains(&cu) {
                return Some(cu);
            }
        }
    }
    for c in name.chars() {
        if !c.is_ascii_alphabetic() { continue; }
        let cl = c.to_ascii_lowercase();
        if !taken.contains(&cl) { return Some(cl); }
        let cu = c.to_ascii_uppercase();
        if !taken.contains(&cu) { return Some(cu); }
    }
    None
}

fn lower_and_validate_flags(
    flags: &[Flag],
    besogne_name_upper: &str,
) -> Result<Vec<ResolvedFlag>, String> {
    let mut by_scope: HashMap<Option<String>, Vec<&Flag>> = HashMap::new();
    for flag in flags {
        by_scope.entry(flag.subcommand.clone()).or_default().push(flag);
    }

    let mut all_env_vars: HashSet<String> = HashSet::new();
    let mut result = Vec::new();
    let builtin_shorts: HashSet<char> = ['l', 'h', 'V'].into_iter().collect();

    for (scope, scope_flags) in &by_scope {
        let mut names: HashSet<String> = HashSet::new();
        let mut shorts_taken: HashSet<char> = builtin_shorts.clone();

        for flag in scope_flags {
            if let Some(s) = flag.short {
                if !shorts_taken.insert(s) {
                    let scope_label = scope.as_deref().unwrap_or("global");
                    return Err(format!(
                        "flag '{}' in scope '{scope_label}': short '-{s}' conflicts",
                        flag.name
                    ));
                }
            }
        }

        for flag in scope_flags {
            let scope_label = scope.as_deref().unwrap_or("global");

            if !names.insert(flag.name.clone()) {
                return Err(format!("duplicate flag name '{}' in scope '{scope_label}'", flag.name));
            }

            let env_var = compute_flag_env_var(flag, besogne_name_upper);
            if !all_env_vars.insert(env_var.clone()) {
                return Err(format!("flag '{}': env var '{env_var}' conflicts", flag.name));
            }

            let short = match flag.kind {
                FlagKind::Positional => None,
                _ => match flag.short {
                    Some(s) => Some(s),
                    None => {
                        let derived = derive_short(&flag.name, &shorts_taken);
                        if let Some(s) = derived { shorts_taken.insert(s); }
                        derived
                    }
                },
            };

            result.push(ResolvedFlag {
                name: flag.name.clone(),
                short,
                description: flag.description.clone(),
                doc: flag.doc.clone(),
                kind: match &flag.kind {
                    FlagKind::Bool => ResolvedFlagKind::Bool,
                    FlagKind::String => ResolvedFlagKind::String,
                    FlagKind::Positional => ResolvedFlagKind::Positional,
                },
                default: flag.default.clone(),
                values: flag.values.clone(),
                required: flag.required.unwrap_or(false),
                env_var,
                subcommand: flag.subcommand.clone(),
            });
        }
    }

    Ok(result)
}

fn resolve_sandbox(sandbox: &Option<manifest::Sandbox>) -> SandboxResolved {
    match sandbox {
        None => SandboxResolved {
            env: EnvSandboxResolved::Inherit,
            tmpdir: false,
            network: NetworkSandboxResolved::Host,
        },
        Some(manifest::Sandbox::Preset(preset)) => match preset {
            manifest::SandboxPreset::None => SandboxResolved {
                env: EnvSandboxResolved::Inherit,
                tmpdir: false,
                network: NetworkSandboxResolved::Host,
            },
            manifest::SandboxPreset::Strict => SandboxResolved {
                env: EnvSandboxResolved::Strict,
                tmpdir: true,
                network: NetworkSandboxResolved::None,
            },
            manifest::SandboxPreset::Container => SandboxResolved {
                env: EnvSandboxResolved::Strict,
                tmpdir: true,
                network: NetworkSandboxResolved::Restricted,
            },
        },
        Some(manifest::Sandbox::Custom(config)) => {
            let base = config
                .preset
                .as_ref()
                .map(|p| resolve_sandbox(&Some(manifest::Sandbox::Preset(p.clone()))))
                .unwrap_or(SandboxResolved {
                    env: EnvSandboxResolved::Inherit,
                    tmpdir: false,
                    network: NetworkSandboxResolved::Host,
                });

            SandboxResolved {
                env: config.env.as_ref().map(|e| match e {
                    manifest::EnvSandbox::Strict => EnvSandboxResolved::Strict,
                    manifest::EnvSandbox::Inherit => EnvSandboxResolved::Inherit,
                }).unwrap_or(base.env),
                tmpdir: config.tmpdir.unwrap_or(base.tmpdir),
                network: config.network.as_ref().map(|n| match n {
                    manifest::NetworkSandbox::None => NetworkSandboxResolved::None,
                    manifest::NetworkSandbox::Host => NetworkSandboxResolved::Host,
                    manifest::NetworkSandbox::Restricted => NetworkSandboxResolved::Restricted,
                }).unwrap_or(base.network),
            }
        }
    }
}
