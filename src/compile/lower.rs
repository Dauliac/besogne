use crate::ir::types::*;
use crate::manifest::{self, Node, Phase, Flag, FlagKind};
use crate::output::style::{self, DiagBuilder};
use std::collections::{HashMap, HashSet};

/// Lower a parsed manifest into the intermediate representation
pub fn lower_manifest(manifest: &manifest::Manifest, manifest_path: &std::path::Path) -> Result<BesogneIR, String> {
    // Version is the content hash of the manifest — deterministic, no user-specified version
    let manifest_json = serde_json::to_vec(manifest)
        .map_err(|e| format!("cannot serialize manifest for hashing: {e}"))?;
    let version = blake3::hash(&manifest_json).to_hex()[..16].to_string();

    let workdir = manifest_path
        .parent()
        .and_then(|p| p.canonicalize().ok())
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|| ".".to_string());

    let metadata = Metadata {
        name: manifest.name.clone(),
        version,
        description: manifest.description.clone(),
        workdir,
    };

    let sandbox = resolve_sandbox(&manifest.sandbox);

    let besogne_name_upper = manifest.name.to_uppercase().replace('-', "_");
    let flags = lower_and_validate_flags(&manifest.flags, &besogne_name_upper)?;

    let mut resolved_nodes = Vec::new();

    // Generate synthetic env inputs for each flag
    for flag in &flags {
        let env_input = ResolvedNode {
            id: ContentId::from_content("env", &flag.env_var, flag.env_var.as_bytes()),
            phase: Phase::Seal,
            node: ResolvedNativeNode::Env {
                name: flag.env_var.clone(),
                value: flag.default.as_ref().and_then(|d| {
                    d.as_str().map(|s| s.to_string()).or_else(|| {
                        d.as_bool().map(|b| if b { "1".to_string() } else { "0".to_string() })
                    })
                }),
                secret: false,
                on_missing: crate::ir::types::OnMissingResolved::Fail,
            },
            parents: vec![],
            from_component: Some("flag".to_string()),
            sealed: None,
        };
        resolved_nodes.push(env_input);
    }

    for (key, input) in &manifest.nodes {
        match input {
            Node::Component(_) => {
                return Err(format!(
                    "input '{key}': component not expanded before lowering (bug in compile pipeline)"
                ));
            }
            _ => {
                let resolved = lower_input(key, input, &metadata.workdir)?;
                resolved_nodes.push(resolved);
            }
        }
    }

    // Resolve exec-phase ordering constraints
    resolve_ordering(&mut resolved_nodes, &manifest.nodes)?;

    // Validate script-as-command patterns: files used as command first args
    validate_script_commands(&resolved_nodes, &metadata.workdir)?;

    Ok(BesogneIR {
        metadata,
        sandbox,
        flags,
        nodes: resolved_nodes,
    })
}

/// Lower a manifest input to IR. The `key` is the map key (= the input's name).
fn lower_input(key: &str, input: &Node, base_workdir: &str) -> Result<ResolvedNode, String> {
    let (native, phase, id) = match input {
        Node::Env(e) => {
            let env_name = e.name.clone().unwrap_or_else(|| key.to_string());
            let on_missing = match e.on_missing.as_ref() {
                Some(crate::manifest::OnMissing::Skip) => crate::ir::types::OnMissingResolved::Skip,
                Some(crate::manifest::OnMissing::Continue) => crate::ir::types::OnMissingResolved::Continue,
                _ => crate::ir::types::OnMissingResolved::Fail,
            };
            let native = ResolvedNativeNode::Env {
                name: env_name.clone(),
                value: e.value.clone(),
                secret: e.secret.unwrap_or(false),
                on_missing,
            };
            let phase = e.phase.clone().unwrap_or(Phase::Seal);
            let id = ContentId::from_content("env", &env_name, env_name.as_bytes());
            (native, phase, id)
        }

        Node::File(f) => {
            let native = ResolvedNativeNode::File {
                path: f.path.clone(),
                expect: f.expect.clone(),
                permissions: f.permissions.clone(),
            };
            let phase = f.phase.clone().unwrap_or(Phase::Seal);
            let id = ContentId::from_content("file", &f.path, f.path.as_bytes());
            (native, phase, id)
        }

        Node::Binary(b) => {
            let bin_name = b.name.clone().unwrap_or_else(|| key.to_string());
            let version_constraint = b.version.clone();
            let parents = b.parents.clone().unwrap_or_default();
            let native = ResolvedNativeNode::Binary {
                name: bin_name.clone(),
                path: b.path.clone(),
                version_constraint,
                parents,
                source: None,
                resolved_path: None,
                resolved_version: None,
                binary_hash: None,
            };
            let phase = b.phase.clone().unwrap_or(Phase::Build);
            // Use the manifest key (not bin_name) as identifier — preserves qualified
            // component paths like "node/toolchain.node" for parent resolution.
            let id = ContentId::from_content("binary", key, key.as_bytes());
            (native, phase, id)
        }

        Node::Service(s) => {
            let identifier = s.tcp.as_deref()
                .or(s.http.as_deref())
                .unwrap_or(key);
            let native = ResolvedNativeNode::Service {
                name: Some(key.to_string()),
                tcp: s.tcp.clone(),
                http: s.http.clone(),
                retry: lower_retry(&s.retry)?,
            };
            let phase = s.phase.clone().unwrap_or(Phase::Exec);
            let id = ContentId::from_content("service", identifier, identifier.as_bytes());
            (native, phase, id)
        }

        Node::Command(c) => {
            let run_resolved = resolve_run_spec(&c.run);
            let cmd_workdir = c.workdir.as_ref().map(|w| {
                let p = std::path::Path::new(base_workdir).join(w);
                p.to_string_lossy().to_string()
            });
            let native = ResolvedNativeNode::Command {
                name: key.to_string(),
                run: run_resolved,
                env: c.env.clone().unwrap_or_default(),
                side_effects: c.side_effects.unwrap_or(false),
                workdir: cmd_workdir,
                force_args: c.force_args.clone().unwrap_or_default(),
                debug_args: c.debug_args.clone().unwrap_or_default(),
                retry: lower_retry(&c.retry)?,
            };
            let phase = c.phase.clone().unwrap_or(Phase::Exec);
            let id = ContentId::from_content("command", key, key.as_bytes());
            (native, phase, id)
        }


        Node::Platform(p) => {
            let identifier = format!(
                "{}-{}",
                p.os.as_deref().unwrap_or("any"),
                p.arch.as_deref().unwrap_or("any")
            );
            let native = ResolvedNativeNode::Platform {
                os: p.os.clone(),
                arch: p.arch.clone(),
            };
            let phase = p.phase.clone().unwrap_or(Phase::Build);
            let id = ContentId::from_content("platform", &identifier, identifier.as_bytes());
            (native, phase, id)
        }

        Node::Dns(d) => {
            let native = ResolvedNativeNode::Dns {
                host: d.host.clone(),
                expect: d.expect.clone(),
                retry: lower_retry(&d.retry)?,
            };
            let phase = d.phase.clone().unwrap_or(Phase::Exec);
            let id = ContentId::from_content("dns", &d.host, d.host.as_bytes());
            (native, phase, id)
        }

        Node::Metric(m) => {
            let native = ResolvedNativeNode::Metric {
                metric: m.metric.clone(),
                path: m.path.clone(),
            };
            let phase = m.phase.clone().unwrap_or(Phase::Exec);
            let id = ContentId::from_content("metric", &m.metric, m.metric.as_bytes());
            (native, phase, id)
        }

        Node::Source(s) => {
            let native = ResolvedNativeNode::Source {
                format: s.format.clone(),
                path: s.path.clone(),
                select: s.select.clone(),
                sealed_env: None,
            };
            let phase = s.phase.clone().unwrap_or(Phase::Seal);
            let id = ContentId::from_content("source", key, key.as_bytes());
            (native, phase, id)
        }

        Node::Std(s) => {
            let native = ResolvedNativeNode::Std {
                stream: s.stream.clone(),
                contains: s.contains.clone().unwrap_or_default(),
                expect: s.expect.clone(),
            };
            let id = ContentId::from_content("std", key, key.as_bytes());
            (native, Phase::Exec, id)
        }

        Node::Component(_) => {
            return Err("components should be expanded before lowering".into());
        }
    };

    Ok(ResolvedNode {
        id,
        phase,
        node: native,
        parents: vec![],
        from_component: None,
        sealed: None,
    })
}

/// Validate that files used as command first arguments are proper executables.
///
/// Detects when a file input is also used as the first word of a command's `run:` —
/// meaning it's actually a binary/script, not just a data file. Validates:
/// 1. The file has the executable bit set
/// 2. The file has a valid shebang
/// 3. The shebang interpreter is declared as a binary input
///
/// This catches the common mistake of declaring a script as `type = "file"` when
/// it's actually executed as a command.
fn validate_script_commands(
    nodes: &[ResolvedNode],
    workdir: &str,
) -> Result<(), String> {
    // Collect file paths (relative → absolute)
    let file_paths: HashMap<String, String> = nodes
        .iter()
        .filter_map(|i| {
            if let ResolvedNativeNode::File { path, .. } = &i.node {
                Some((path.clone(), path.clone()))
            } else {
                None
            }
        })
        .collect();

    // Collect declared binary names
    let binary_names: HashSet<String> = nodes
        .iter()
        .filter_map(|i| {
            if let ResolvedNativeNode::Binary { name, .. } = &i.node {
                Some(name.clone())
            } else {
                None
            }
        })
        .collect();

    // Collect command first words
    let mut errors = Vec::new();
    let mut warnings = Vec::new();

    for node in nodes {
        if let ResolvedNativeNode::Command { name, run, .. } = &node.node {
            if run.is_empty() {
                continue;
            }
            let first_word = &run[0];

            // Check if the command first word matches a file input path
            // Normalize: "./level1.sh" matches "level1.sh" and vice versa
            let normalized_first = first_word
                .strip_prefix("./")
                .unwrap_or(first_word);

            let matched_file = file_paths.keys().find(|fp| {
                let normalized_fp = fp.strip_prefix("./").unwrap_or(fp);
                normalized_fp == normalized_first || *fp == first_word
            });

            if let Some(file_path) = matched_file {
                let abs_path = std::path::Path::new(workdir).join(file_path);

                // Check executable bit
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    if let Ok(meta) = std::fs::metadata(&abs_path) {
                        let mode = meta.permissions().mode();
                        if mode & 0o111 == 0 {
                            let header = style::error_diag(&format!(
                                "command {}: file '{}' is used as a command but is not executable",
                                style::bold(name), file_path));
                            let body = DiagBuilder::new()
                                .location(&format!("manifest [nodes.{name}]"))
                                .blank()
                                .code(&format!("run = [\"{file_path}\"]"))
                                .blank()
                                .hint(&format!("chmod +x {file_path}  (make it executable)"))
                                .hint(&format!("or use run = [\"sh\", \"{file_path}\"]  (and declare 'sh' as a binary node)"))
                                .build();
                            errors.push(format!("{header}\n{body}"));
                            continue;
                        }
                    }
                }

                // Check shebang
                if let Ok(content) = std::fs::read_to_string(&abs_path) {
                    if let Some(first_line) = content.lines().next() {
                        if first_line.starts_with("#!") {
                            let shebang = first_line.trim_start_matches("#!");
                            let shebang = shebang.trim();

                            // Parse interpreter from shebang
                            // Common forms: "#!/bin/sh", "#!/usr/bin/env bash", "#!/usr/bin/python3"
                            let interpreter = if shebang.starts_with("/usr/bin/env ") || shebang.starts_with("/bin/env ") {
                                // env form: take the command after env
                                shebang.split_whitespace().nth(1)
                            } else {
                                // Direct path: take basename
                                shebang.split_whitespace().next()
                                    .and_then(|p| p.rsplit('/').next())
                            };

                            if let Some(interp) = interpreter {
                                if !binary_names.contains(interp) {
                                    let header = style::error_diag(&format!(
                                        "command {}: script '{}' uses undeclared interpreter '{interp}'",
                                        style::bold(name), file_path));
                                    let body = DiagBuilder::new()
                                        .location(&format!("manifest [nodes.{name}]"))
                                        .blank()
                                        .code(&format!("#!{shebang}"))
                                        .blank()
                                        .hint(&format!("add [nodes.{interp}] with type = \"binary\" to your manifest"))
                                        .build();
                                    errors.push(format!("{header}\n{body}"));
                                }
                            }
                        } else {
                            // No shebang — warn
                            let header = style::warning_diag(&format!(
                                "command {}: file '{}' has no shebang (#!)",
                                style::bold(name), file_path));
                            let body = DiagBuilder::new()
                                .location(&format!("manifest [nodes.{name}]"))
                                .hint(&format!("add a shebang line (e.g., #!/bin/sh) to '{file_path}'"))
                                .build();
                            warnings.push(format!("{header}\n{body}"));
                        }
                    }
                }

                // Advise about .sh extension
                if file_path.ends_with(".sh") || file_path.ends_with(".bash") {
                    let header = style::warning_diag(&format!(
                        "command {}: '{}' is used as an executable",
                        style::bold(name), file_path));
                    let body = DiagBuilder::new()
                        .location(&format!("manifest [nodes.{name}]"))
                        .hint("consider removing the .sh extension (executables don't need file extensions)")
                        .build();
                    warnings.push(format!("{header}\n{body}"));
                }
            }
        }
    }

    // Print warnings (non-fatal)
    for w in &warnings {
        eprintln!("{w}");
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "script validation failed:\n  {}",
            errors.join("\n  ")
        ))
    }
}

/// Resolve the run spec (was: exec) into a flat command vec
fn resolve_run_spec(spec: &manifest::ExecSpec) -> Vec<String> {
    match spec {
        manifest::ExecSpec::Array(args) => args.clone(),
        manifest::ExecSpec::Shell(s) => vec!["sh".into(), "-c".into(), s.clone()],
        manifest::ExecSpec::Script { file, args } => {
            let mut cmd = vec![file.clone()];
            if let Some(a) = args {
                cmd.extend(a.clone());
            }
            cmd
        }
    }
}

/// Resolve `parents:` ordering constraints for exec-phase inputs
fn resolve_ordering(
    nodes: &mut Vec<ResolvedNode>,
    manifest_nodes: &HashMap<String, manifest::Node>,
) -> Result<(), String> {
    // Build name→id map for exec-phase inputs + source nodes (any phase)
    let name_to_id: HashMap<String, ContentId> = nodes
        .iter()
        .filter_map(|i| {
            match &i.node {
                ResolvedNativeNode::Command { name, .. } if i.phase == Phase::Exec => {
                    Some((name.clone(), i.id.clone()))
                }
                ResolvedNativeNode::Service { name: Some(name), .. } if i.phase == Phase::Exec => {
                    Some((name.clone(), i.id.clone()))
                }
                ResolvedNativeNode::Source { .. } => {
                    let key = i.id.0.split(':').nth(1).unwrap_or("").to_string();
                    Some((key, i.id.clone()))
                }
                ResolvedNativeNode::Std { .. } if i.phase == Phase::Exec => {
                    let key = i.id.0.split(':').nth(1).unwrap_or("").to_string();
                    Some((key, i.id.clone()))
                }
                ResolvedNativeNode::Dns { .. } if i.phase == Phase::Exec => {
                    let key = i.id.0.split(':').nth(1).unwrap_or("").to_string();
                    Some((key, i.id.clone()))
                }
                _ => None,
            }
        })
        .collect();

    // Collect `parents` constraints from manifest (key = input name)
    let mut parents_by_name: HashMap<String, Vec<String>> = HashMap::new();
    for (key, mi) in manifest_nodes {
        match mi {
            manifest::Node::Command(c) => {
                if let Some(parents) = &c.parents {
                    parents_by_name.insert(key.clone(), parents.clone());
                }
            }
            manifest::Node::Service(s) => {
                if let Some(parents) = &s.parents {
                    parents_by_name.insert(key.clone(), parents.clone());
                }
            }
            manifest::Node::Source(s) => {
                if let Some(parents) = &s.parents {
                    parents_by_name.insert(key.clone(), parents.clone());
                }
            }
            manifest::Node::Std(s) => {
                if let Some(parents) = &s.parents {
                    parents_by_name.insert(key.clone(), parents.clone());
                }
            }
            manifest::Node::File(f) => {
                if let Some(parents) = &f.parents {
                    parents_by_name.insert(key.clone(), parents.clone());
                }
            }
            manifest::Node::Dns(d) => {
                if let Some(parents) = &d.parents {
                    parents_by_name.insert(key.clone(), parents.clone());
                }
            }
            _ => {}
        }
    }

    // Resolve string refs → ContentIds
    let resolutions: Vec<(usize, Vec<ContentId>)> = nodes
        .iter()
        .enumerate()
        .filter(|(_, i)| i.phase == Phase::Exec || matches!(&i.node, ResolvedNativeNode::Source { .. }))
        .filter_map(|(idx, input)| {
            let cmd_name = match &input.node {
                ResolvedNativeNode::Command { name, .. } => Some(name.as_str()),
                ResolvedNativeNode::Service { name: Some(name), .. } => Some(name.as_str()),
                ResolvedNativeNode::Source { .. } | ResolvedNativeNode::Std { .. }
                | ResolvedNativeNode::Dns { .. } => {
                    Some(input.id.0.split(':').nth(1).unwrap_or(""))
                }
                _ => None,
            }?;

            let parent_names = parents_by_name.get(cmd_name)?;
            let resolved: Result<Vec<ContentId>, String> = parent_names
                .iter()
                .map(|dep_name| {
                    name_to_id.get(dep_name).cloned().ok_or_else(|| {
                        let suggestion = closest_match(dep_name, name_to_id.keys());
                        let mut msg = format!(
                            "node '{cmd_name}' has parents: ['{dep_name}'] which is not a resolvable node"
                        );
                        if let Some(closest) = suggestion {
                            msg.push_str(&format!("\n   = hint: did you mean '{closest}'?"));
                        }
                        msg
                    })
                })
                .collect();

            match resolved {
                Ok(ids) => Some(Ok((idx, ids))),
                Err(e) => Some(Err(e)),
            }
        })
        .collect::<Result<Vec<_>, _>>()?;

    for (idx, parent_ids) in resolutions {
        nodes[idx].parents = parent_ids;
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

/// Find the closest matching name within edit distance 2.
fn closest_match<'a, I>(target: &str, candidates: I) -> Option<String>
where
    I: IntoIterator<Item = &'a String>,
{
    candidates
        .into_iter()
        .filter_map(|c| {
            let d = levenshtein(target, c);
            if d <= 2 { Some((d, c.clone())) } else { None }
        })
        .min_by_key(|(d, _)| *d)
        .map(|(_, name)| name)
}

/// Levenshtein edit distance between two strings.
fn levenshtein(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let (m, n) = (a.len(), b.len());
    let mut prev = (0..=n).collect::<Vec<_>>();
    let mut curr = vec![0; n + 1];
    for i in 1..=m {
        curr[0] = i;
        for j in 1..=n {
            let cost = if a[i - 1] == b[j - 1] { 0 } else { 1 };
            curr[j] = (prev[j] + 1)
                .min(curr[j - 1] + 1)
                .min(prev[j - 1] + cost);
        }
        std::mem::swap(&mut prev, &mut curr);
    }
    prev[n]
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

/// Parse a human-readable duration string into milliseconds.
/// Supported: "500ms", "1s", "2m", "1h", "1m30s"
fn parse_duration_proper(s: &str) -> Result<u64, String> {
    let s = s.trim();
    let mut total_ms: u64 = 0;
    let mut rest = s;

    while !rest.is_empty() {
        // Consume digits
        let num_end = rest.find(|c: char| !c.is_ascii_digit() && c != '.').unwrap_or(rest.len());
        if num_end == 0 {
            return Err(format!("invalid duration: '{s}'"));
        }
        let val: f64 = rest[..num_end].parse().map_err(|_| format!("invalid duration: '{s}'"))?;
        rest = &rest[num_end..];

        if rest.is_empty() {
            // Bare number = seconds
            total_ms += (val * 1_000.0) as u64;
            break;
        }

        // Consume unit
        if rest.starts_with("ms") {
            total_ms += val as u64;
            rest = &rest[2..];
        } else if rest.starts_with('s') {
            total_ms += (val * 1_000.0) as u64;
            rest = &rest[1..];
        } else if rest.starts_with('m') {
            total_ms += (val * 60_000.0) as u64;
            rest = &rest[1..];
        } else if rest.starts_with('h') {
            total_ms += (val * 3_600_000.0) as u64;
            rest = &rest[1..];
        } else {
            return Err(format!("unknown duration unit in '{s}'"));
        }
    }

    if total_ms == 0 && s != "0" && s != "0s" && s != "0ms" {
        return Err(format!("duration parsed to zero: '{s}'"));
    }

    Ok(total_ms)
}

/// Lower manifest RetryConfig to IR RetryResolved
fn lower_retry(retry: &Option<manifest::RetryConfig>) -> Result<Option<RetryResolved>, String> {
    let rc = match retry {
        Some(r) => r,
        None => return Ok(None),
    };

    let interval_ms = parse_duration_proper(&rc.interval)?;

    let backoff = match rc.backoff.as_deref() {
        None | Some("fixed") => RetryBackoff::Fixed,
        Some("linear") => RetryBackoff::Linear,
        Some("exponential") | Some("expo") => RetryBackoff::Exponential,
        Some(other) => return Err(format!(
            "unknown backoff strategy '{other}': expected fixed, linear, or exponential"
        )),
    };

    let max_interval_ms = rc.max_interval.as_ref()
        .map(|s| parse_duration_proper(s))
        .transpose()?;

    let timeout_ms = rc.timeout.as_ref()
        .map(|s| parse_duration_proper(s))
        .transpose()?;

    if rc.attempts < 1 {
        return Err("retry.attempts must be >= 1".into());
    }

    Ok(Some(RetryResolved {
        attempts: rc.attempts,
        interval_ms,
        backoff,
        max_interval_ms,
        timeout_ms,
    }))
}

/// Validate node compositions: reject invalid parent→child relationships.
///
/// Rule: **Only `Command` nodes can have `Std` children.**
/// A `std` node probes command stdout/stderr/exit_code — it makes no sense
/// as a child of env, file, binary, service, etc.
fn validate_node_compositions(nodes: &[ResolvedNode]) -> Result<(), String> {
    let node_by_id: HashMap<&ContentId, &ResolvedNode> = nodes.iter()
        .map(|n| (&n.id, n)).collect();

    let mut errors = Vec::new();

    for node in nodes {
        if let ResolvedNativeNode::Std { stream, .. } = &node.node {
            let node_key = node.id.0.split(':').nth(1).unwrap_or("?");

            for parent_id in &node.parents {
                if let Some(parent) = node_by_id.get(parent_id) {
                    if !matches!(&parent.node, ResolvedNativeNode::Command { .. }) {
                        let parent_type = match &parent.node {
                            ResolvedNativeNode::Env { .. } => "env",
                            ResolvedNativeNode::File { .. } => "file",
                            ResolvedNativeNode::Binary { .. } => "binary",
                            ResolvedNativeNode::Service { .. } => "service",
                            ResolvedNativeNode::Dns { .. } => "dns",
                            ResolvedNativeNode::Metric { .. } => "metric",
                            ResolvedNativeNode::Platform { .. } => "platform",
                            ResolvedNativeNode::Source { .. } => "source",
                            ResolvedNativeNode::Std { .. } => "std",
                            ResolvedNativeNode::Command { .. } => unreachable!(),
                        };
                        let parent_key = parent.id.0.split(':').nth(1).unwrap_or("?");
                        let header = style::error_diag(&format!(
                            "std node '{}' cannot be child of {} node '{}'",
                            node_key, parent_type, parent_key,
                        ));
                        let body = DiagBuilder::new()
                            .location(&format!("manifest [nodes.{node_key}]"))
                            .blank()
                            .code(&format!("stream = \"{stream}\""))
                            .code(&format!("parents = [\"{parent_key}\"]"))
                            .blank()
                            .note(&format!("'{parent_type}' nodes do not produce stdout/stderr/exit_code"))
                            .hint("std nodes can only be children of command nodes")
                            .build();
                        errors.push(format!("{header}\n{body}"));
                    }
                }
            }
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("\n\n"))
    }
}
