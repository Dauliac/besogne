use crate::ir::types::*;
use crate::manifest::{self, Node, Phase, Flag, FlagKind};
use crate::output::style::{self, DiagBuilder};
use std::collections::{HashMap, HashSet};

/// Lower a parsed manifest into the intermediate representation
pub fn lower_manifest(manifest: &manifest::Manifest, manifest_path: &std::path::Path) -> Result<BesogneIR, crate::error::BesogneError> {
    // Version is the content hash of the manifest — deterministic, no user-specified version.
    // Serialize to canonical JSON with sorted keys to avoid HashMap ordering non-determinism.
    let manifest_json = canonical_json(manifest)
        .map_err(|e| crate::error::BesogneError::Compile(format!("cannot serialize manifest for hashing: {e}")))?;
    let version = blake3::hash(&manifest_json).to_hex()[..16].to_string();

    let workdir = manifest_path
        .parent()
        .and_then(|p| p.canonicalize().ok())
        .map(|p| p.to_string_lossy().to_string())
        // In Nix sandbox or store context, the manifest dir is ephemeral.
        // Use "." so the sealed binary runs from caller's CWD at runtime.
        .filter(|p| !p.starts_with("/nix/store") && !p.starts_with("/build"))
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
        let on_missing = if flag.required {
            crate::ir::types::OnMissingResolved::Fail
        } else {
            crate::ir::types::OnMissingResolved::Continue
        };
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
                on_missing,
                merge: crate::ir::types::EnvMergeResolved::Override,
                separator: ":".to_string(),
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
                return Err(crate::error::BesogneError::Compile(format!(
                    "input '{key}': component not expanded before lowering (bug in compile pipeline)"
                )));
            }
            _ => {
                let resolved = lower_input(key, input, &metadata.workdir, &besogne_name_upper)?;
                resolved_nodes.push(resolved);
            }
        }
    }

    // Resolve exec-phase ordering constraints
    resolve_ordering(&mut resolved_nodes, &manifest.nodes)?;

    // Validate script-as-command patterns: files used as command first args
    validate_script_commands(&resolved_nodes, &metadata.workdir)?;

    // Validate node composition rules (e.g., std must be child of command)
    validate_node_compositions(&resolved_nodes)?;

    // Static $VAR checking: warn about unresolved variable references
    validate_var_refs(&resolved_nodes);

    // Sort nodes by content ID for deterministic serialization.
    // HashMap iteration order is non-deterministic — without sorting,
    // the same manifest produces different IR JSON → different binary hash.
    resolved_nodes.sort_by(|a, b| a.id.0.cmp(&b.id.0));

    Ok(BesogneIR {
        metadata,
        sandbox,
        flags,
        nodes: resolved_nodes,
    })
}

/// Lower a manifest input to IR. The `key` is the map key (= the input's name).
fn lower_input(key: &str, input: &Node, base_workdir: &str, besogne_name_upper: &str) -> Result<ResolvedNode, crate::error::BesogneError> {
    let (native, phase, id) = match input {
        Node::Env(e) => {
            let env_name = e.name.clone().unwrap_or_else(|| key.to_string());
            let on_missing = match e.on_missing.as_ref() {
                Some(crate::manifest::OnMissing::Skip) => crate::ir::types::OnMissingResolved::Skip,
                Some(crate::manifest::OnMissing::Continue) => crate::ir::types::OnMissingResolved::Continue,
                _ => crate::ir::types::OnMissingResolved::Fail,
            };
            let merge = match e.merge.as_ref() {
                Some(crate::manifest::EnvMerge::Prepend) => crate::ir::types::EnvMergeResolved::Prepend,
                Some(crate::manifest::EnvMerge::Append) => crate::ir::types::EnvMergeResolved::Append,
                Some(crate::manifest::EnvMerge::Fallback) => crate::ir::types::EnvMergeResolved::Fallback,
                _ => crate::ir::types::EnvMergeResolved::Override,
            };
            let native = ResolvedNativeNode::Env {
                name: env_name.clone(),
                value: e.value.clone(),
                secret: e.secret.unwrap_or(false),
                on_missing,
                merge,
                separator: e.separator.clone().unwrap_or_else(|| ":".to_string()),
            };
            let phase = e.phase.clone().unwrap_or(Phase::Seal);
            // Use manifest key for content ID to ensure uniqueness when multiple
            // env nodes bind the same variable name (e.g., merge strategies)
            let id = ContentId::from_content("env", key, key.as_bytes());
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
            eprintln!("{}", crate::output::style::warning_diag(
                &format!("native 'service' type is deprecated for node '{key}'. Use tcp/check or http/check component instead")));

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
                verify: c.verify,
                resources: ResourceLimits {
                    priority: c.priority.as_ref().map(resolve_priority).unwrap_or_default(),
                    memory_limit: c.memory_limit.as_ref().map(|s| parse_byte_size(s)),
                },
                hide_output: c.hide_output.unwrap_or(false),
            };
            let phase = c.phase.clone().unwrap_or(Phase::Exec);
            let id = ContentId::from_content("command", key, key.as_bytes());
            (native, phase, id)
        }


        Node::Platform(p) => {
            eprintln!("{}", crate::output::style::warning_diag(
                &format!("native 'platform' type is deprecated for node '{key}'. Use system/platform component instead")));
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
            eprintln!("{}", crate::output::style::warning_diag(
                &format!("native 'dns' type is deprecated for node '{key}'. Use dns/resolve component instead")));
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
            eprintln!("{}", crate::output::style::warning_diag(
                &format!("native 'metric' type is deprecated for node '{key}'. Use system/cpu-count or system/memory-mb component instead")));
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
            let phase = s.phase.clone().unwrap_or(Phase::Exec);
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

        Node::Flag(f) => {
            let flag_name = f.name.clone().unwrap_or_else(|| key.to_string());
            let flag_upper = flag_name.to_uppercase().replace('-', "_");
            let env_var = format!("{besogne_name_upper}_{flag_upper}");
            let native = ResolvedNativeNode::Flag {
                name: flag_name,
                env_var,
                value: f.value.clone(),
                on_missing: crate::ir::types::OnMissingResolved::Skip,
            };
            let phase = f.phase.clone().unwrap_or(Phase::Exec);
            let id = ContentId::from_content("flag", key, key.as_bytes());
            (native, phase, id)
        }

        Node::Component(_) => {
            return Err(crate::error::BesogneError::Compile("components should be expanded before lowering".into()));
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
) -> Result<(), crate::error::BesogneError> {
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
        Err(crate::error::BesogneError::Compile(format!(
            "script validation failed:\n  {}",
            errors.join("\n  ")
        )))
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
) -> Result<(), crate::error::BesogneError> {
    // Build name→id map for exec-phase inputs + source nodes (any phase)
    let mut name_to_id: HashMap<String, ContentId> = nodes
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
                ResolvedNativeNode::File { .. } if i.phase == Phase::Exec => {
                    let key = i.id.0.split(':').nth(1).unwrap_or("").to_string();
                    Some((key, i.id.clone()))
                }
                ResolvedNativeNode::Flag { .. } => {
                    let key = i.id.0.split(':').nth(1).unwrap_or("").to_string();
                    Some((key, i.id.clone()))
                }
                ResolvedNativeNode::Env { .. } if i.phase == Phase::Exec => {
                    // Index by manifest key (from content ID)
                    let key = i.id.0.split(':').nth(1).unwrap_or("").to_string();
                    Some((key, i.id.clone()))
                }
                _ => None,
            }
        })
        .collect();

    // Add manifest key aliases for nodes whose content ID key differs from manifest key.
    // This ensures parent references by manifest key resolve correctly.
    for (key, mi) in manifest_nodes {
        let content_key = match mi {
            manifest::Node::Env(e) => Some(e.name.clone().unwrap_or_else(|| key.clone())),
            manifest::Node::File(f) => Some(f.path.clone()),
            manifest::Node::Binary(b) => Some(b.name.clone().unwrap_or_else(|| key.clone())),
            _ => None,
        };
        if let Some(ck) = content_key {
            if *key != ck {
                if let Some(id) = name_to_id.get(&ck) {
                    name_to_id.insert(key.clone(), id.clone());
                }
            }
        }
    }

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
                    // Also index by path (content ID uses path, not manifest key)
                    if f.path != *key {
                        parents_by_name.insert(f.path.clone(), parents.clone());
                    }
                }
            }
            manifest::Node::Dns(d) => {
                if let Some(parents) = &d.parents {
                    parents_by_name.insert(key.clone(), parents.clone());
                }
            }
            manifest::Node::Flag(f) => {
                if let Some(parents) = &f.parents {
                    parents_by_name.insert(key.clone(), parents.clone());
                }
            }
            manifest::Node::Env(e) => {
                if let Some(parents) = &e.parents {
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
                | ResolvedNativeNode::Dns { .. } | ResolvedNativeNode::File { .. }
                | ResolvedNativeNode::Flag { .. } | ResolvedNativeNode::Env { .. } => {
                    Some(input.id.0.split(':').nth(1).unwrap_or(""))
                }
                _ => None,
            }?;

            let parent_names = parents_by_name.get(cmd_name)?;
            let resolved: Result<Vec<ContentId>, crate::error::BesogneError> = parent_names
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
                        crate::error::BesogneError::Compile(msg)
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

#[allow(dead_code)]
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
) -> Result<Vec<ResolvedFlag>, crate::error::BesogneError> {
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
                    return Err(crate::error::BesogneError::Compile(format!(
                        "flag '{}' in scope '{scope_label}': short '-{s}' conflicts",
                        flag.name
                    )));
                }
            }
        }

        for flag in scope_flags {
            let scope_label = scope.as_deref().unwrap_or("global");

            if !names.insert(flag.name.clone()) {
                return Err(crate::error::BesogneError::Compile(format!("duplicate flag name '{}' in scope '{scope_label}'", flag.name)));
            }

            let env_var = compute_flag_env_var(flag, besogne_name_upper);
            if !all_env_vars.insert(env_var.clone()) {
                return Err(crate::error::BesogneError::Compile(format!("flag '{}': env var '{env_var}' conflicts", flag.name)));
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
            priority: PriorityResolved::Normal,
            memory_limit: None,
        },
        Some(manifest::Sandbox::Preset(preset)) => match preset {
            manifest::SandboxPreset::None => SandboxResolved {
                env: EnvSandboxResolved::Inherit,
                tmpdir: false,
                network: NetworkSandboxResolved::Host,
                priority: PriorityResolved::Normal,
                memory_limit: None,
            },
            manifest::SandboxPreset::Strict => SandboxResolved {
                env: EnvSandboxResolved::Strict,
                tmpdir: true,
                network: NetworkSandboxResolved::None,
                priority: PriorityResolved::Normal,
                memory_limit: None,
            },
            manifest::SandboxPreset::Container => SandboxResolved {
                env: EnvSandboxResolved::Strict,
                tmpdir: true,
                network: NetworkSandboxResolved::Restricted,
                priority: PriorityResolved::Normal,
                memory_limit: None,
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
                    priority: PriorityResolved::Normal,
                    memory_limit: None,
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
                priority: config.priority.as_ref().map(resolve_priority).unwrap_or(base.priority),
                memory_limit: config.memory_limit.as_ref().map(|s| parse_byte_size(s)).or(base.memory_limit),
            }
        }
    }
}

fn resolve_priority(p: &manifest::Priority) -> PriorityResolved {
    match p {
        manifest::Priority::Normal => PriorityResolved::Normal,
        manifest::Priority::Low => PriorityResolved::Low,
        manifest::Priority::Background => PriorityResolved::Background,
    }
}

/// Parse a human-readable byte size: "512MB", "2GB", "1024KB"
fn parse_byte_size(s: &str) -> u64 {
    let s = s.trim();
    let (num_str, unit) = s.split_at(s.find(|c: char| c.is_alphabetic()).unwrap_or(s.len()));
    let num: f64 = num_str.trim().parse().unwrap_or(0.0);
    match unit.trim().to_uppercase().as_str() {
        "KB" | "K" => (num * 1024.0) as u64,
        "MB" | "M" => (num * 1024.0 * 1024.0) as u64,
        "GB" | "G" => (num * 1024.0 * 1024.0 * 1024.0) as u64,
        "TB" | "T" => (num * 1024.0 * 1024.0 * 1024.0 * 1024.0) as u64,
        _ => num as u64, // bare number = bytes
    }
}

/// Parse a human-readable duration string into milliseconds.
/// Supported: "500ms", "1s", "2m", "1h", "1m30s"
fn parse_duration_proper(s: &str) -> Result<u64, crate::error::BesogneError> {
    let s = s.trim();
    let mut total_ms: u64 = 0;
    let mut rest = s;

    while !rest.is_empty() {
        // Consume digits
        let num_end = rest.find(|c: char| !c.is_ascii_digit() && c != '.').unwrap_or(rest.len());
        if num_end == 0 {
            return Err(crate::error::BesogneError::Compile(format!("invalid duration: '{s}'")));
        }
        let val: f64 = rest[..num_end]
            .parse()
            .map_err(|_| crate::error::BesogneError::Compile(format!("invalid duration: '{s}'")))?;
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
            return Err(crate::error::BesogneError::Compile(format!("unknown duration unit in '{s}'")));
        }
    }

    if total_ms == 0 && s != "0" && s != "0s" && s != "0ms" {
        return Err(crate::error::BesogneError::Compile(format!("duration parsed to zero: '{s}'")));
    }

    Ok(total_ms)
}

/// Lower manifest RetryConfig to IR RetryResolved
fn lower_retry(retry: &Option<manifest::RetryConfig>) -> Result<Option<RetryResolved>, crate::error::BesogneError> {
    let rc = match retry {
        Some(r) => r,
        None => return Ok(None),
    };

    let interval_ms = parse_duration_proper(&rc.interval)?;

    let backoff = match rc.backoff.as_deref() {
        None | Some("fixed") => RetryBackoff::Fixed,
        Some("linear") => RetryBackoff::Linear,
        Some("exponential") | Some("expo") => RetryBackoff::Exponential,
        Some(other) => return Err(crate::error::BesogneError::Compile(format!(
            "unknown backoff strategy '{other}': expected fixed, linear, or exponential"
        ))),
    };

    let max_interval_ms = rc.max_interval.as_ref()
        .map(|s| parse_duration_proper(s))
        .transpose()?;

    let timeout_ms = rc.timeout.as_ref()
        .map(|s| parse_duration_proper(s))
        .transpose()?;

    if rc.attempts < 1 {
        return Err(crate::error::BesogneError::Compile("retry.attempts must be >= 1".into()));
    }

    Ok(Some(RetryResolved {
        attempts: rc.attempts,
        interval_ms,
        backoff,
        max_interval_ms,
        timeout_ms,
    }))
}

/// Validate node compositions: reject impossible relationships.
///
/// Rules enforced:
/// 1. `std` stream must be one of: stdout, stderr, exit_code, stdin
/// 2. `std(stdout/stderr/exit_code)` must have exactly 1 command parent
/// 3. `std(stdin)` must have 0 parents (stdin comes from binary input)
/// 4. `source` must be exec phase (needs DAG ordering for env var delivery)
fn validate_node_compositions(nodes: &[ResolvedNode]) -> Result<(), crate::error::BesogneError> {
    let node_by_id: HashMap<&ContentId, &ResolvedNode> = nodes.iter()
        .map(|n| (&n.id, n)).collect();

    let mut errors = Vec::new();
    let valid_streams = ["stdout", "stderr", "exit_code", "stdin"];

    for node in nodes {
        let node_key = node.id.0.split(':').nth(1).unwrap_or("?");

        match &node.node {
            ResolvedNativeNode::Std { stream, .. } => {
                // Rule 1: stream must be a known value
                if !valid_streams.contains(&stream.as_str()) {
                    let header = style::error_diag(&format!(
                        "unknown stream '{stream}' in std node '{node_key}'",
                    ));
                    let body = DiagBuilder::new()
                        .location(&format!("manifest [nodes.{node_key}]"))
                        .blank()
                        .code(&format!("stream = \"{stream}\""))
                        .blank()
                        .note("valid streams are: stdout, stderr, exit_code, stdin")
                        .build();
                    errors.push(format!("{header}\n{body}"));
                    continue;
                }

                if stream == "stdin" {
                    // Rule 3: stdin std must have 0 parents
                    if !node.parents.is_empty() {
                        let parent_keys: Vec<String> = node.parents.iter()
                            .map(|pid| node_by_id.get(pid)
                                .map(|p| p.id.0.split(':').nth(1).unwrap_or("?").to_string())
                                .unwrap_or_else(|| "?".to_string()))
                            .collect();
                        let header = style::error_diag(&format!(
                            "stdin std node '{node_key}' cannot have parents",
                        ));
                        let body = DiagBuilder::new()
                            .location(&format!("manifest [nodes.{node_key}]"))
                            .blank()
                            .code("stream = \"stdin\"")
                            .code(&format!("parents = {:?}", parent_keys))
                            .blank()
                            .note("stdin comes from binary input, not from a parent node")
                            .hint("remove the parents field from this stdin node")
                            .build();
                        errors.push(format!("{header}\n{body}"));
                    }
                } else {
                    // stdout/stderr/exit_code rules

                    // Rule 2a: must have at least 1 parent
                    if node.parents.is_empty() {
                        let header = style::error_diag(&format!(
                            "std node '{node_key}' has no parents",
                        ));
                        let body = DiagBuilder::new()
                            .location(&format!("manifest [nodes.{node_key}]"))
                            .blank()
                            .code(&format!("stream = \"{stream}\""))
                            .blank()
                            .note(&format!("'{stream}' must be captured from a command"))
                            .hint("add parents = [\"<command-name>\"] to specify the source command")
                            .build();
                        errors.push(format!("{header}\n{body}"));
                        continue;
                    }

                    // Rule 2b: all parents must be commands
                    let mut command_count = 0;
                    for parent_id in &node.parents {
                        if let Some(parent) = node_by_id.get(parent_id) {
                            if matches!(&parent.node, ResolvedNativeNode::Command { .. }) {
                                command_count += 1;
                            } else {
                                let parent_type = node_type_name(&parent.node);
                                let parent_key = parent.id.0.split(':').nth(1).unwrap_or("?");
                                let header = style::error_diag(&format!(
                                    "std node '{node_key}' cannot be child of {parent_type} node '{parent_key}'",
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

                    // Rule 2c: exactly 1 command parent (no ambiguity)
                    if command_count > 1 {
                        let header = style::error_diag(&format!(
                            "std node '{node_key}' has {command_count} command parents (ambiguous)",
                        ));
                        let body = DiagBuilder::new()
                            .location(&format!("manifest [nodes.{node_key}]"))
                            .blank()
                            .code(&format!("stream = \"{stream}\""))
                            .blank()
                            .note(&format!("cannot determine which command's {stream} to capture"))
                            .hint("std nodes must have exactly one command parent")
                            .build();
                        errors.push(format!("{header}\n{body}"));
                    }
                }
            }

            ResolvedNativeNode::Source { .. } => {
                // Rule 4: source must be exec phase
                if node.phase != Phase::Exec {
                    let phase_str = match node.phase {
                        Phase::Build => "build",
                        Phase::Seal => "seal",
                        Phase::Exec => unreachable!(),
                    };
                    let header = style::error_diag(&format!(
                        "source node '{node_key}' must be exec phase (found '{phase_str}')",
                    ));
                    let body = DiagBuilder::new()
                        .location(&format!("manifest [nodes.{node_key}]"))
                        .blank()
                        .code(&format!("phase = \"{phase_str}\""))
                        .blank()
                        .note("source nodes participate in the exec DAG for proper ordering")
                        .hint("remove the phase field (defaults to exec) or set phase = \"exec\"")
                        .build();
                    errors.push(format!("{header}\n{body}"));
                }
            }

            _ => {}
        }
    }

    // Rule 5: Typed edge validation — reject invalid parent→child type combinations
    for node in nodes {
        let node_key = node.id.0.split(':').nth(1).unwrap_or("?");
        let child_type = node_type_name(&node.node);

        for parent_id in &node.parents {
            let Some(parent) = node_by_id.get(parent_id) else { continue };
            let parent_type = node_type_name(&parent.node);
            let parent_key = parent.id.0.split(':').nth(1).unwrap_or("?");

            let invalid = match (&parent.node, &node.node) {
                // command → env: Record cannot coerce to String without Extract (std)
                (ResolvedNativeNode::Command { .. }, ResolvedNativeNode::Env { .. }) => {
                    Some("command produces a Record — use a std node to extract stdout/stderr first")
                }
                // command → source: Record cannot be parsed without Extract (std)
                (ResolvedNativeNode::Command { .. }, ResolvedNativeNode::Source { .. }) => {
                    Some("command produces a Record — use a std node to extract stdout first, then parse with source")
                }
                // source → std: Map cannot be extracted like a Record
                (ResolvedNativeNode::Source { .. }, ResolvedNativeNode::Std { .. }) => {
                    Some("source produces a Map (key-value bindings), not a Record — std can only extract from command output")
                }
                _ => None,
            };

            if let Some(hint) = invalid {
                let header = style::error_diag(&format!(
                    "invalid edge: {parent_type} '{parent_key}' → {child_type} '{node_key}'"
                ));
                let body = DiagBuilder::new()
                    .location(&format!("manifest [nodes.{node_key}]"))
                    .blank()
                    .code(&format!("parents = [\"{parent_key}\"]"))
                    .blank()
                    .note(hint)
                    .build();
                errors.push(format!("{header}\n{body}"));
            }
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(crate::error::BesogneError::Compile(errors.join("\n\n")))
    }
}

/// Static $VAR checking: for each command, extract $VAR references and verify
/// each has a binding ancestor (env/source/flag). Emits warnings for unresolved refs.
fn validate_var_refs(nodes: &[ResolvedNode]) {
    let node_by_id: HashMap<&ContentId, &ResolvedNode> = nodes.iter()
        .map(|n| (&n.id, n)).collect();

    // Collect all variable names provided by env/source/flag nodes
    // Also build a set of seal-phase env names (globally visible)
    let mut seal_env_names: HashSet<String> = HashSet::new();
    for node in nodes {
        if node.phase != Phase::Exec {
            if let ResolvedNativeNode::Env { name, .. } = &node.node {
                seal_env_names.insert(name.clone());
            }
        }
    }

    // Well-known shell/system vars that should never be warned about
    let system_vars: HashSet<&str> = [
        "HOME", "USER", "PATH", "SHELL", "TERM", "LANG", "LC_ALL",
        "PWD", "OLDPWD", "HOSTNAME", "LOGNAME", "TMPDIR", "TMP", "TEMP",
        "XDG_CACHE_HOME", "XDG_CONFIG_HOME", "XDG_DATA_HOME", "XDG_RUNTIME_DIR",
        "DISPLAY", "WAYLAND_DISPLAY", "SSH_AUTH_SOCK", "EDITOR", "VISUAL",
        "PAGER", "LESS", "GREP_OPTIONS", "COLUMNS", "LINES",
        "IFS", "OPTARG", "OPTIND", "PPID", "UID", "EUID", "RANDOM",
        "LINENO", "SECONDS", "BASH", "BASH_VERSION", "ZSH_VERSION",
        "?", "!", "#", "@", "*", "0", "1", "2", "3", "4", "5", "6", "7", "8", "9",
    ].iter().copied().collect();

    for node in nodes {
        if let ResolvedNativeNode::Command { name, run, env, .. } = &node.node {
            // Extract $VAR refs from run args
            let var_refs: HashSet<String> = run.iter()
                .flat_map(|arg| extract_cmd_env_refs(arg))
                .collect();

            if var_refs.is_empty() { continue; }

            // Collect vars available from ancestors (DAG-scoped)
            let mut ancestor_vars: HashSet<String> = HashSet::new();

            // Walk ancestors
            let mut stack: Vec<&ContentId> = node.parents.iter().collect();
            let mut visited = HashSet::new();
            while let Some(pid) = stack.pop() {
                if !visited.insert(pid) { continue; }
                if let Some(parent) = node_by_id.get(pid) {
                    match &parent.node {
                        ResolvedNativeNode::Env { name: env_name, .. } => {
                            ancestor_vars.insert(env_name.clone());
                        }
                        ResolvedNativeNode::Source { .. } => {
                            // Source nodes inject multiple vars — we can't know which at compile time
                            // unless they have a sealed_env. Mark as "provides any var".
                            // For now, if a source is an ancestor, skip all warnings for this command.
                            ancestor_vars.insert("__source_wildcard__".to_string());
                        }
                        ResolvedNativeNode::Flag { env_var, .. } => {
                            ancestor_vars.insert(env_var.clone());
                        }
                        _ => {}
                    }
                    for gpid in &parent.parents {
                        stack.push(gpid);
                    }
                }
            }

            // If a source ancestor exists, skip warnings (source injects unknown vars)
            if ancestor_vars.contains("__source_wildcard__") { continue; }

            // Command-level env: overrides
            let cmd_env_keys: HashSet<String> = env.keys().cloned().collect();

            // Check each $VAR ref
            let mut unresolved: Vec<&String> = var_refs.iter()
                .filter(|var| {
                    !seal_env_names.contains(var.as_str())
                        && !ancestor_vars.contains(var.as_str())
                        && !cmd_env_keys.contains(var.as_str())
                        && !system_vars.contains(var.as_str())
                })
                .collect();

            if !unresolved.is_empty() {
                unresolved.sort();
                let vars_str = unresolved.iter().map(|v| format!("${v}")).collect::<Vec<_>>().join(", ");
                eprintln!("{}", crate::output::style::warning_diag(
                    &format!("command '{name}' references {vars_str} but no ancestor provides it")
                ));
                eprintln!("  {}", crate::output::style::dim(
                    "add env nodes as parents, or these may come from the system environment"
                ));
            }
        }
    }
}

/// Extract $VAR references from a string (command arg).
fn extract_cmd_env_refs(s: &str) -> Vec<String> {
    let mut vars = Vec::new();
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '$' {
            if chars.peek() == Some(&'{') {
                chars.next();
                let var: String = chars.by_ref().take_while(|&c| c != '}').collect();
                let name = var.split(|c: char| !c.is_alphanumeric() && c != '_')
                    .next().unwrap_or("");
                if !name.is_empty() && name.chars().next().map(|c| c.is_alphabetic() || c == '_').unwrap_or(false) {
                    vars.push(name.to_string());
                }
            } else {
                let var: String = chars.by_ref()
                    .take_while(|c| c.is_alphanumeric() || *c == '_').collect();
                if !var.is_empty() && var.chars().next().map(|c| c.is_alphabetic() || c == '_').unwrap_or(false) {
                    vars.push(var);
                }
            }
        }
    }
    vars
}

fn node_type_name(node: &ResolvedNativeNode) -> &'static str {
    match node {
        ResolvedNativeNode::Env { .. } => "env",
        ResolvedNativeNode::File { .. } => "file",
        ResolvedNativeNode::Binary { .. } => "binary",
        ResolvedNativeNode::Service { .. } => "service",
        ResolvedNativeNode::Dns { .. } => "dns",
        ResolvedNativeNode::Metric { .. } => "metric",
        ResolvedNativeNode::Platform { .. } => "platform",
        ResolvedNativeNode::Source { .. } => "source",
        ResolvedNativeNode::Std { .. } => "std",
        ResolvedNativeNode::Command { .. } => "command",
        ResolvedNativeNode::Flag { .. } => "flag",
    }
}

/// Serialize any value to canonical JSON with sorted keys.
/// Ensures deterministic output regardless of HashMap iteration order.
fn canonical_json<T: serde::Serialize>(value: &T) -> Result<Vec<u8>, serde_json::Error> {
    let v = serde_json::to_value(value)?;
    let sorted = sort_json_value(v);
    serde_json::to_vec(&sorted)
}

fn sort_json_value(v: serde_json::Value) -> serde_json::Value {
    match v {
        serde_json::Value::Object(map) => {
            let mut sorted: Vec<(String, serde_json::Value)> = map.into_iter()
                .map(|(k, v)| (k, sort_json_value(v)))
                .collect();
            sorted.sort_by(|a, b| a.0.cmp(&b.0));
            serde_json::Value::Object(sorted.into_iter().collect())
        }
        serde_json::Value::Array(arr) => {
            serde_json::Value::Array(arr.into_iter().map(sort_json_value).collect())
        }
        other => other,
    }
}
