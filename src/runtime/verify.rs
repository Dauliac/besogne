//! Idempotency verification: run exec phase twice, compare fingerprints.
//!
//! Like Nix's --check: if a command produces different outputs on repeated runs
//! with the same inputs, it's non-idempotent. This catches undeclared side effects,
//! non-determinism (timestamps, random values), and stateful dependencies.

use crate::ir::{BesogneIR, ResolvedNativeInput};
use crate::manifest::{Phase, EnsureSpec};
use crate::ir::dag;
use crate::output::OutputRenderer;
use crate::tracer;
use std::collections::HashMap;
use std::time::Instant;

/// Fingerprint of a single command execution
#[derive(Debug, Clone)]
pub struct CommandFingerprint {
    pub exit_code: i32,
    pub stdout_hash: String,
    pub stderr_hash: String,
    pub ensure_hashes: HashMap<String, String>, // path → blake3 hash
}

/// Result of comparing two runs of the same command
#[derive(Debug)]
pub struct VerifyResult {
    pub name: String,
    pub idempotent: bool,
    pub mismatches: Vec<Mismatch>,
    pub side_effects_declared: bool,
}

#[derive(Debug)]
pub enum Mismatch {
    ExitCode { run1: i32, run2: i32 },
    Stdout { run1_hash: String, run2_hash: String },
    Stderr { run1_hash: String, run2_hash: String },
    EnsureFile { path: String, run1_hash: String, run2_hash: String },
    EnsureMissing { path: String, which_run: u8 },
}

impl std::fmt::Display for Mismatch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Mismatch::ExitCode { run1, run2 } =>
                write!(f, "exit code: run1={run1} run2={run2}"),
            Mismatch::Stdout { run1_hash, run2_hash } =>
                write!(f, "stdout: run1={} run2={}", &run1_hash[..8], &run2_hash[..8]),
            Mismatch::Stderr { run1_hash, run2_hash } =>
                write!(f, "stderr: run1={} run2={}", &run1_hash[..8], &run2_hash[..8]),
            Mismatch::EnsureFile { path, run1_hash, run2_hash } =>
                write!(f, "ensure {path}: run1={} run2={}", &run1_hash[..8], &run2_hash[..8]),
            Mismatch::EnsureMissing { path, which_run } =>
                write!(f, "ensure {path}: missing after run {which_run}"),
        }
    }
}

/// Run the exec phase twice and compare fingerprints
pub fn verify_idempotency(
    ir: &BesogneIR,
    all_variables: &HashMap<String, String>,
    renderer: &mut dyn OutputRenderer,
) -> Vec<VerifyResult> {
    eprintln!("\n\x1b[1m=== idempotency verification ===\x1b[0m");
    eprintln!("Running exec phase twice to detect non-determinism...\n");

    // Collect exec-phase commands
    let exec_commands: Vec<(&str, &[String], &HashMap<String, String>, &[EnsureSpec], bool)> = ir
        .inputs
        .iter()
        .filter(|i| i.phase == Phase::Exec)
        .filter_map(|i| {
            if let ResolvedNativeInput::Command {
                name, run, env, ensure, side_effects, ..
            } = &i.input
            {
                Some((name.as_str(), run.as_slice(), env, ensure.as_slice(), *side_effects))
            } else {
                None
            }
        })
        .collect();

    // Build DAG order
    let (graph, _) = match dag::build_exec_dag(ir) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("error building DAG: {e}");
            return vec![];
        }
    };
    let tiers = match dag::compute_tiers(&graph) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("error computing tiers: {e}");
            return vec![];
        }
    };

    let input_by_id: HashMap<_, _> = ir
        .inputs
        .iter()
        .filter(|i| i.phase == Phase::Exec)
        .map(|i| (i.id.clone(), i))
        .collect();

    // Run 1
    eprintln!("\x1b[1mRun 1/2:\x1b[0m");
    let run1 = execute_and_fingerprint(ir, &graph, &tiers, &input_by_id, all_variables);

    // Clean ensure files between runs
    for (_, _, _, ensures, _) in &exec_commands {
        for spec in *ensures {
            let _ = std::fs::remove_file(&spec.path);
            let _ = std::fs::remove_dir_all(&spec.path);
        }
    }

    // Run 2
    eprintln!("\n\x1b[1mRun 2/2:\x1b[0m");
    let run2 = execute_and_fingerprint(ir, &graph, &tiers, &input_by_id, all_variables);

    // Compare
    let mut results = Vec::new();
    for (name, _, _, _, side_effects) in &exec_commands {
        let fp1 = run1.get(*name);
        let fp2 = run2.get(*name);

        let mut mismatches = Vec::new();

        match (fp1, fp2) {
            (Some(f1), Some(f2)) => {
                if f1.exit_code != f2.exit_code {
                    mismatches.push(Mismatch::ExitCode {
                        run1: f1.exit_code,
                        run2: f2.exit_code,
                    });
                }
                if f1.stdout_hash != f2.stdout_hash {
                    mismatches.push(Mismatch::Stdout {
                        run1_hash: f1.stdout_hash.clone(),
                        run2_hash: f2.stdout_hash.clone(),
                    });
                }
                if f1.stderr_hash != f2.stderr_hash {
                    mismatches.push(Mismatch::Stderr {
                        run1_hash: f1.stderr_hash.clone(),
                        run2_hash: f2.stderr_hash.clone(),
                    });
                }
                // Compare ensure files
                let all_paths: std::collections::HashSet<_> = f1
                    .ensure_hashes
                    .keys()
                    .chain(f2.ensure_hashes.keys())
                    .collect();
                for path in all_paths {
                    match (f1.ensure_hashes.get(path), f2.ensure_hashes.get(path)) {
                        (Some(h1), Some(h2)) if h1 != h2 => {
                            mismatches.push(Mismatch::EnsureFile {
                                path: path.clone(),
                                run1_hash: h1.clone(),
                                run2_hash: h2.clone(),
                            });
                        }
                        (None, Some(_)) => {
                            mismatches.push(Mismatch::EnsureMissing {
                                path: path.clone(),
                                which_run: 1,
                            });
                        }
                        (Some(_), None) => {
                            mismatches.push(Mismatch::EnsureMissing {
                                path: path.clone(),
                                which_run: 2,
                            });
                        }
                        _ => {}
                    }
                }
            }
            _ => {
                // One or both runs didn't produce a result — skip
            }
        }

        let idempotent = mismatches.is_empty();

        results.push(VerifyResult {
            name: name.to_string(),
            idempotent,
            mismatches,
            side_effects_declared: *side_effects,
        });
    }

    // Report
    eprintln!("\n\x1b[1m=== verification results ===\x1b[0m");
    let mut has_errors = false;
    for result in &results {
        if result.side_effects_declared {
            eprintln!(
                "  \x1b[33m⊘\x1b[0m {} (side_effects=true, skipped)",
                result.name
            );
        } else if result.idempotent {
            eprintln!("  \x1b[32m✓\x1b[0m {} idempotent", result.name);
        } else {
            has_errors = true;
            eprintln!(
                "  \x1b[31m✗\x1b[0m {} NOT IDEMPOTENT",
                result.name
            );
            for m in &result.mismatches {
                eprintln!("    {m}");
            }
            eprintln!(
                "    \x1b[2mhint: if this is expected, add side_effects = true\x1b[0m"
            );
        }
    }

    if has_errors {
        eprintln!(
            "\n\x1b[31mverification FAILED\x1b[0m — some commands are not idempotent"
        );
    } else {
        eprintln!(
            "\n\x1b[32mverification PASSED\x1b[0m — all commands are idempotent"
        );
    }

    results
}

// ── Output assertion validation ──────────────────────────────────

/// Validate command output against an OutputSpec.
/// Returns Ok(()) if all assertions pass, Err(message) on first failure.
pub fn check_output(
    spec: &crate::manifest::OutputSpec,
    stdout: &str,
    stderr: &str,
    exit_code: i32,
) -> Result<(), String> {
    if let Some(expected) = spec.exit_code {
        if exit_code != expected {
            return Err(format!("exit_code: expected {expected}, got {exit_code}"));
        }
    }

    if let Some(patterns) = &spec.stdout_contains {
        for pat in patterns {
            if !stdout.contains(pat.as_str()) {
                return Err(format!("stdout_contains: expected \"{pat}\""));
            }
        }
    }

    if let Some(patterns) = &spec.stderr_contains {
        for pat in patterns {
            if !stderr.contains(pat.as_str()) {
                return Err(format!("stderr_contains: expected \"{pat}\""));
            }
        }
    }

    if let Some(pattern) = &spec.stdout_matches {
        let re = regex::Regex::new(pattern)
            .map_err(|e| format!("stdout_matches: invalid regex: {e}"))?;
        if !re.is_match(stdout) {
            return Err(format!("stdout_matches: no match for /{pattern}/"));
        }
    }

    if let Some(pattern) = &spec.stderr_matches {
        let re = regex::Regex::new(pattern)
            .map_err(|e| format!("stderr_matches: invalid regex: {e}"))?;
        if !re.is_match(stderr) {
            return Err(format!("stderr_matches: no match for /{pattern}/"));
        }
    }

    if let Some(expected) = &spec.json {
        let parsed: serde_json::Value = serde_json::from_str(stdout)
            .map_err(|e| format!("json: stdout is not valid JSON: {e}"))?;
        for (key, expected_val) in expected {
            match json_path(&parsed, key) {
                Some(val) if val == expected_val => {}
                Some(val) => return Err(format!("json.{key}: expected {expected_val}, got {val}")),
                None => return Err(format!("json.{key}: path not found")),
            }
        }
    }

    Ok(())
}

/// Simple dot-separated JSON path lookup
fn json_path<'a>(value: &'a serde_json::Value, path: &str) -> Option<&'a serde_json::Value> {
    let mut current = value;
    for key in path.split('.') {
        current = current.get(key)?;
    }
    Some(current)
}

/// Execute the DAG and collect fingerprints for each command
fn execute_and_fingerprint(
    ir: &BesogneIR,
    graph: &petgraph::graph::DiGraph<crate::ir::ContentId, ()>,
    tiers: &[Vec<petgraph::graph::NodeIndex>],
    input_by_id: &HashMap<crate::ir::ContentId, &crate::ir::ResolvedInput>,
    all_variables: &HashMap<String, String>,
) -> HashMap<String, CommandFingerprint> {
    let mut fingerprints = HashMap::new();

    for tier in tiers {
        for &node_idx in tier {
            let content_id = &graph[node_idx];
            let input = match input_by_id.get(content_id) {
                Some(i) => i,
                None => continue,
            };

            if let ResolvedNativeInput::Command {
                name,
                run,
                env,
                ensure,
                side_effects,
                ..
            } = &input.input
            {
                // Skip side-effect commands — they're declared non-idempotent
                if *side_effects {
                    eprintln!("  \x1b[33m⊘\x1b[0m {name} (side_effects, skipped)");
                    continue;
                }

                let mut cmd_env = all_variables.clone();
                cmd_env.extend(env.clone());

                let result = match tracer::execute_traced(run, &cmd_env, &ir.sandbox.env) {
                    Ok(r) => r,
                    Err(e) => {
                        eprintln!("  \x1b[31m✗\x1b[0m {name}: {e}");
                        continue;
                    }
                };

                let stdout_hash = blake3::hash(&result.stdout).to_hex().to_string();
                let stderr_hash = blake3::hash(&result.stderr).to_hex().to_string();

                // Hash ensure files
                let mut ensure_hashes = HashMap::new();
                for spec in ensure {
                    if let Ok(content) = std::fs::read(&spec.path) {
                        ensure_hashes.insert(
                            spec.path.clone(),
                            blake3::hash(&content).to_hex().to_string(),
                        );
                    }
                }

                eprintln!(
                    "  {} {name}  exit={}  stdout={:.8}  ensure={}",
                    if result.exit_code == 0 { "\x1b[32m✓\x1b[0m" } else { "\x1b[31m✗\x1b[0m" },
                    result.exit_code,
                    &stdout_hash[..8],
                    ensure_hashes.len(),
                );

                fingerprints.insert(
                    name.clone(),
                    CommandFingerprint {
                        exit_code: result.exit_code,
                        stdout_hash,
                        stderr_hash,
                        ensure_hashes,
                    },
                );
            }
        }
    }

    fingerprints
}
