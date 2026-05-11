//! Idempotency verification: run exec phase twice, compare fingerprints.
//!
//! Like Nix's --check: if a command produces different outputs on repeated runs
//! with the same inputs, it's non-idempotent.
//!
//! On first run (automatic): warn only, show diff.
//! On explicit --verify: fail hard, show diff.

use crate::ir::{BesogneIR, ResolvedNativeInput};
use crate::manifest::{Phase, EnsureSpec};
use crate::ir::dag;
use crate::output::OutputRenderer;
use crate::tracer;
use similar::{ChangeTag, TextDiff};
use std::collections::HashMap;

/// Fingerprint of a single command execution
#[derive(Debug, Clone)]
pub struct CommandFingerprint {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
    pub stdout_hash: String,
    pub stderr_hash: String,
    pub ensure_hashes: HashMap<String, String>,
}

/// Result of comparing two runs
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
    Stdout { run1: String, run2: String },
    Stderr { run1: String, run2: String },
    EnsureFile { path: String, run1_hash: String, run2_hash: String },
    EnsureMissing { path: String, which_run: u8 },
}

/// Run exec phase twice and compare. Returns results + the output from run2
/// (which is the "real" run if idempotent).
pub fn verify_idempotency(
    ir: &BesogneIR,
    all_variables: &HashMap<String, String>,
    renderer: &mut dyn OutputRenderer,
    json_mode: bool,
) -> Vec<VerifyResult> {
    let (graph, _) = match dag::build_exec_dag(ir) {
        Ok(d) => d,
        Err(e) => { eprintln!("error: {e}"); return vec![]; }
    };
    let tiers = match dag::compute_tiers(&graph) {
        Ok(t) => t,
        Err(e) => { eprintln!("error: {e}"); return vec![]; }
    };

    let input_by_id: HashMap<_, _> = ir.inputs.iter()
        .filter(|i| i.phase == Phase::Exec)
        .map(|i| (i.id.clone(), i))
        .collect();

    // Collect command info
    let exec_commands: Vec<(&str, &[EnsureSpec], bool)> = ir.inputs.iter()
        .filter(|i| i.phase == Phase::Exec)
        .filter_map(|i| {
            if let ResolvedNativeInput::Command { name, ensure, side_effects, .. } = &i.input {
                Some((name.as_str(), ensure.as_slice(), *side_effects))
            } else { None }
        })
        .collect();

    eprintln!("\x1b[2mbesogne: running idempotency check (exec phase x2)...\x1b[0m");

    // Run 1
    let run1 = execute_and_fingerprint(ir, &graph, &tiers, &input_by_id, all_variables);

    // Clean ensure files
    for (_, ensures, _) in &exec_commands {
        for spec in *ensures {
            let _ = std::fs::remove_file(&spec.path);
            let _ = std::fs::remove_dir_all(&spec.path);
        }
    }

    // Run 2
    let run2 = execute_and_fingerprint(ir, &graph, &tiers, &input_by_id, all_variables);

    // Compare and build results
    let mut results = Vec::new();
    for (name, _, side_effects) in &exec_commands {
        if *side_effects {
            results.push(VerifyResult {
                name: name.to_string(),
                idempotent: true,
                mismatches: vec![],
                side_effects_declared: true,
            });
            continue;
        }

        let fp1 = run1.get(*name);
        let fp2 = run2.get(*name);
        let mut mismatches = Vec::new();

        if let (Some(f1), Some(f2)) = (fp1, fp2) {
            if f1.exit_code != f2.exit_code {
                mismatches.push(Mismatch::ExitCode { run1: f1.exit_code, run2: f2.exit_code });
            }
            if f1.stdout_hash != f2.stdout_hash {
                mismatches.push(Mismatch::Stdout {
                    run1: f1.stdout.clone(), run2: f2.stdout.clone(),
                });
            }
            if f1.stderr_hash != f2.stderr_hash {
                mismatches.push(Mismatch::Stderr {
                    run1: f1.stderr.clone(), run2: f2.stderr.clone(),
                });
            }
            // Compare ensure files
            let all_paths: std::collections::HashSet<_> = f1.ensure_hashes.keys()
                .chain(f2.ensure_hashes.keys()).collect();
            for path in all_paths {
                match (f1.ensure_hashes.get(path), f2.ensure_hashes.get(path)) {
                    (Some(h1), Some(h2)) if h1 != h2 => {
                        mismatches.push(Mismatch::EnsureFile {
                            path: path.clone(), run1_hash: h1.clone(), run2_hash: h2.clone(),
                        });
                    }
                    (None, Some(_)) => mismatches.push(Mismatch::EnsureMissing { path: path.clone(), which_run: 1 }),
                    (Some(_), None) => mismatches.push(Mismatch::EnsureMissing { path: path.clone(), which_run: 2 }),
                    _ => {}
                }
            }
        }

        results.push(VerifyResult {
            name: name.to_string(),
            idempotent: mismatches.is_empty(),
            mismatches,
            side_effects_declared: false,
        });
    }

    // Display results
    display_verify_results(&results, &run1, &run2, json_mode);

    results
}

/// Display verification results with smart output
fn display_verify_results(
    results: &[VerifyResult],
    run1: &HashMap<String, CommandFingerprint>,
    run2: &HashMap<String, CommandFingerprint>,
    json_mode: bool,
) {
    for result in results {
        if result.side_effects_declared {
            if json_mode {
                println!("{}", serde_json::json!({
                    "event": "verify",
                    "command": result.name,
                    "status": "skipped",
                    "reason": "side_effects"
                }));
            } else {
                eprintln!("  \x1b[33m⊘\x1b[0m {} \x1b[2m(side_effects, skipped)\x1b[0m", result.name);
            }
        } else if result.idempotent {
            // Show output ONCE (since both runs match)
            if json_mode {
                let fp = run2.get(&result.name);
                println!("{}", serde_json::json!({
                    "event": "verify",
                    "command": result.name,
                    "status": "idempotent",
                    "stdout": fp.map(|f| f.stdout.as_str()).unwrap_or(""),
                }));
            } else {
                eprintln!("  \x1b[32m✓\x1b[0m {} \x1b[32midempotent\x1b[0m", result.name);
                // Show output once (from run2)
                if let Some(fp) = run2.get(&result.name) {
                    if !fp.stdout.trim().is_empty() {
                        for line in fp.stdout.lines().take(5) {
                            eprintln!("    \x1b[2m{line}\x1b[0m");
                        }
                    }
                }
            }
        } else {
            // NOT IDEMPOTENT — show diff
            if json_mode {
                display_mismatch_json(&result.name, &result.mismatches);
            } else {
                eprintln!("  \x1b[31m✗\x1b[0m {} \x1b[31mNOT IDEMPOTENT\x1b[0m", result.name);
                display_mismatch_human(&result.mismatches);
            }
        }
    }
}

/// Maximum diff lines to show inline. Beyond this, suggest --verify for full output.
const MAX_DIFF_LINES: usize = 20;

/// Human-readable colored diff output
fn display_mismatch_human(mismatches: &[Mismatch]) {
    // Check if any mismatch involves an ensure (validated output) vs just stdout/stderr
    let has_ensure_mismatch = mismatches.iter().any(|m| matches!(m, Mismatch::EnsureFile { .. } | Mismatch::EnsureMissing { .. }));
    let has_output_mismatch = mismatches.iter().any(|m| matches!(m, Mismatch::Stdout { .. } | Mismatch::Stderr { .. }));

    for m in mismatches {
        match m {
            Mismatch::ExitCode { run1, run2 } => {
                eprintln!("    \x1b[31mexit code differs: run1={run1} run2={run2}\x1b[0m");
            }
            Mismatch::Stdout { run1, run2 } => {
                let diff = TextDiff::from_lines(run1.as_str(), run2.as_str());
                let diff_lines: Vec<_> = diff.iter_all_changes().collect();
                let changed = diff_lines.iter().filter(|c| c.tag() != ChangeTag::Equal).count();

                if changed == 0 {
                    continue;
                }

                if !has_ensure_mismatch {
                    eprintln!("    \x1b[33mstdout differs (not validated by ensure — likely not important)\x1b[0m");
                } else {
                    eprintln!("    \x1b[31mstdout differs:\x1b[0m");
                }

                if changed > MAX_DIFF_LINES {
                    eprintln!("    \x1b[2m({changed} lines differ — run with --verify to see full diff)\x1b[0m");
                } else {
                    print_colored_diff(run1, run2);
                }
            }
            Mismatch::Stderr { run1, run2 } => {
                let diff = TextDiff::from_lines(run1.as_str(), run2.as_str());
                let changed = diff.iter_all_changes().filter(|c| c.tag() != ChangeTag::Equal).count();

                if changed == 0 { continue; }

                if !has_ensure_mismatch {
                    eprintln!("    \x1b[33mstderr differs (not validated by ensure — likely not important)\x1b[0m");
                } else {
                    eprintln!("    \x1b[31mstderr differs:\x1b[0m");
                }

                if changed > MAX_DIFF_LINES {
                    eprintln!("    \x1b[2m({changed} lines differ — run with --verify to see full diff)\x1b[0m");
                } else {
                    print_colored_diff(run1, run2);
                }
            }
            Mismatch::EnsureFile { path, run1_hash, run2_hash } => {
                eprintln!(
                    "    \x1b[31mensure {path} differs: {} vs {} (validated output changed!)\x1b[0m",
                    &run1_hash[..8], &run2_hash[..8]
                );
            }
            Mismatch::EnsureMissing { path, which_run } => {
                eprintln!("    \x1b[31mensure {path}: missing after run {which_run}\x1b[0m");
            }
        }
    }

    if has_output_mismatch && !has_ensure_mismatch {
        eprintln!("    \x1b[2mnote: only stdout/stderr differ (no ensure outputs) — this may be acceptable\x1b[0m");
        eprintln!("    \x1b[2mhint: add side_effects = true to silence, or add ensure to validate outputs\x1b[0m");
    } else {
        eprintln!("    \x1b[2mhint: add side_effects = true if this is expected\x1b[0m");
    }
}

/// Colored inline diff using `similar`
fn print_colored_diff(old: &str, new: &str) {
    let diff = TextDiff::from_lines(old, new);
    for change in diff.iter_all_changes() {
        match change.tag() {
            ChangeTag::Delete => eprint!("    \x1b[31m- {change}\x1b[0m"),
            ChangeTag::Insert => eprint!("    \x1b[32m+ {change}\x1b[0m"),
            ChangeTag::Equal => eprint!("    \x1b[2m  {change}\x1b[0m"),
        }
    }
}

/// JSON diff output (unified diff format)
fn display_mismatch_json(name: &str, mismatches: &[Mismatch]) {
    for m in mismatches {
        match m {
            Mismatch::Stdout { run1, run2 } => {
                let diff = TextDiff::from_lines(run1, run2);
                let unified = diff.unified_diff()
                    .header("run1/stdout", "run2/stdout")
                    .to_string();
                println!("{}", serde_json::json!({
                    "event": "verify",
                    "command": name,
                    "status": "mismatch",
                    "field": "stdout",
                    "diff": unified,
                }));
            }
            Mismatch::Stderr { run1, run2 } => {
                let diff = TextDiff::from_lines(run1, run2);
                let unified = diff.unified_diff()
                    .header("run1/stderr", "run2/stderr")
                    .to_string();
                println!("{}", serde_json::json!({
                    "event": "verify",
                    "command": name,
                    "status": "mismatch",
                    "field": "stderr",
                    "diff": unified,
                }));
            }
            Mismatch::ExitCode { run1, run2 } => {
                println!("{}", serde_json::json!({
                    "event": "verify",
                    "command": name,
                    "status": "mismatch",
                    "field": "exit_code",
                    "run1": run1,
                    "run2": run2,
                }));
            }
            Mismatch::EnsureFile { path, run1_hash, run2_hash } => {
                println!("{}", serde_json::json!({
                    "event": "verify",
                    "command": name,
                    "status": "mismatch",
                    "field": "ensure",
                    "path": path,
                    "run1_hash": run1_hash,
                    "run2_hash": run2_hash,
                }));
            }
            Mismatch::EnsureMissing { path, which_run } => {
                println!("{}", serde_json::json!({
                    "event": "verify",
                    "command": name,
                    "status": "mismatch",
                    "field": "ensure_missing",
                    "path": path,
                    "missing_run": which_run,
                }));
            }
        }
    }
}

/// Display one-liner when idempotency was already verified
pub fn display_already_verified() {
    eprintln!("\x1b[2m✓ idempotency verified\x1b[0m");
}

/// Check output assertions
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

fn json_path<'a>(value: &'a serde_json::Value, path: &str) -> Option<&'a serde_json::Value> {
    let mut current = value;
    for key in path.split('.') { current = current.get(key)?; }
    Some(current)
}

/// Execute DAG and collect fingerprints
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
                name, run, env, ensure, side_effects, workdir, ..
            } = &input.input {
                if *side_effects { continue; }

                let mut cmd_env = all_variables.clone();
                cmd_env.extend(env.clone());

                let result = match tracer::execute_traced(run, &cmd_env, &ir.sandbox.env, workdir.as_deref()) {
                    Ok(r) => r,
                    Err(_) => continue,
                };

                let stdout = String::from_utf8_lossy(&result.stdout).to_string();
                let stderr = String::from_utf8_lossy(&result.stderr).to_string();

                let mut ensure_hashes = HashMap::new();
                for spec in ensure {
                    if let Ok(content) = std::fs::read(&spec.path) {
                        ensure_hashes.insert(spec.path.clone(), blake3::hash(&content).to_hex().to_string());
                    }
                }

                fingerprints.insert(name.clone(), CommandFingerprint {
                    exit_code: result.exit_code,
                    stdout_hash: blake3::hash(stdout.as_bytes()).to_hex().to_string(),
                    stderr_hash: blake3::hash(stderr.as_bytes()).to_hex().to_string(),
                    stdout,
                    stderr,
                    ensure_hashes,
                });
            }
        }
    }
    fingerprints
}
