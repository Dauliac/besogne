//! Idempotency verification: re-run commands, diff only declared outputs.
//!
//! Only cache-relevant properties are compared:
//! - Exit code
//! - Child `std` node content (stdout/stderr probed by declared nodes)
//! - Child `file` node content hashes (postconditions)
//!
//! NOT compared: PIDs, timestamps, metrics, process tree, raw stdout/stderr
//! (unless a `std` node explicitly captures them).

use crate::output::style;
use crate::ir::{ResolvedNode, ResolvedNativeNode, ContentId};
use std::collections::HashMap;

pub struct VerifyResult {
    pub command_name: String,
    pub idempotent: bool,
    pub diffs: Vec<NodeDiff>,
}

pub struct NodeDiff {
    pub label: String,
    pub kind: DiffKind,
}

pub enum DiffKind {
    ExitCode { run1: i32, run2: i32 },
    FileHash { run1: String, run2: String },
    StdContent { run1: String, run2: String },
    ExecFailed { error: String },
}

pub(crate) enum OutputValue {
    ExitCode(i32),
    FileHash(String),
    StdContent(String),
}

/// Collect only declared child node values for a command.
pub(crate) fn collect_declared_outputs(
    command_name: &str,
    command_id: &ContentId,
    ir_nodes: &[ResolvedNode],
    stdout: &str,
    stderr: &str,
    exit_code: i32,
) -> HashMap<String, OutputValue> {
    let mut outputs = HashMap::new();
    outputs.insert(
        format!("exit_code of {command_name}"),
        OutputValue::ExitCode(exit_code),
    );

    for node in ir_nodes {
        if !node.parents.contains(command_id) { continue; }
        match &node.node {
            ResolvedNativeNode::Std { stream, .. } => {
                let content = match stream.as_str() {
                    "stdout" => stdout.to_string(),
                    "stderr" => stderr.to_string(),
                    "exit_code" => exit_code.to_string(),
                    _ => continue,
                };
                outputs.insert(format!("std:{stream} of {command_name}"), OutputValue::StdContent(content));
            }
            ResolvedNativeNode::File { path, .. } => {
                let hash = std::fs::read(path)
                    .map(|b| blake3::hash(&b).to_hex()[..16].to_string())
                    .unwrap_or_else(|_| "<missing>".into());
                outputs.insert(format!("file:{path}"), OutputValue::FileHash(hash));
            }
            _ => {}
        }
    }
    outputs
}

pub(crate) fn diff_outputs(run1: &HashMap<String, OutputValue>, run2: &HashMap<String, OutputValue>) -> Vec<NodeDiff> {
    let mut diffs = Vec::new();
    for (label, v1) in run1 {
        let Some(v2) = run2.get(label) else {
            diffs.push(NodeDiff { label: label.clone(), kind: DiffKind::ExecFailed { error: "missing in run 2".into() } });
            continue;
        };
        match (v1, v2) {
            (OutputValue::ExitCode(e1), OutputValue::ExitCode(e2)) if e1 != e2 => {
                diffs.push(NodeDiff { label: label.clone(), kind: DiffKind::ExitCode { run1: *e1, run2: *e2 } });
            }
            (OutputValue::FileHash(h1), OutputValue::FileHash(h2)) if h1 != h2 => {
                diffs.push(NodeDiff { label: label.clone(), kind: DiffKind::FileHash { run1: h1.clone(), run2: h2.clone() } });
            }
            (OutputValue::StdContent(c1), OutputValue::StdContent(c2)) if c1 != c2 => {
                diffs.push(NodeDiff { label: label.clone(), kind: DiffKind::StdContent { run1: c1.clone(), run2: c2.clone() } });
            }
            _ => {}
        }
    }
    diffs
}

/// Re-run a command and diff declared child node values.
pub fn verify_command(
    command_name: &str,
    command_id: &ContentId,
    run: &[String],
    env: &HashMap<String, String>,
    sandbox: &crate::ir::EnvSandboxResolved,
    workdir: Option<&str>,
    first_result: &crate::tracer::CommandResult,
    ir_nodes: &[ResolvedNode],
) -> VerifyResult {
    let stdout1 = String::from_utf8_lossy(&first_result.stdout).to_string();
    let stderr1 = String::from_utf8_lossy(&first_result.stderr).to_string();
    let outputs1 = collect_declared_outputs(command_name, command_id, ir_nodes, &stdout1, &stderr1, first_result.exit_code);

    let result2 = match crate::tracer::execute_traced(run, env, sandbox, workdir, &crate::ir::ResourceLimits::default()) {
        Ok(r) => r,
        Err(e) => {
            return VerifyResult {
                command_name: command_name.to_string(), idempotent: false,
                diffs: vec![NodeDiff { label: "execution".into(), kind: DiffKind::ExecFailed { error: e.to_string() } }],
            };
        }
    };

    let stdout2 = String::from_utf8_lossy(&result2.stdout).to_string();
    let stderr2 = String::from_utf8_lossy(&result2.stderr).to_string();
    let outputs2 = collect_declared_outputs(command_name, command_id, ir_nodes, &stdout2, &stderr2, result2.exit_code);

    let diffs = diff_outputs(&outputs1, &outputs2);
    VerifyResult { command_name: command_name.to_string(), idempotent: diffs.is_empty(), diffs }
}

/// Format verification result for human display.
pub fn format_verify_human(result: &VerifyResult) {
    if result.idempotent {
        eprintln!("  {} {} {}",
            style::styled(style::diagnostic::IDEMPOTENT, "\u{2713}"),
            result.command_name,
            style::styled(style::diagnostic::IDEMPOTENT, style::message::IDEMPOTENT));
        return;
    }

    eprintln!("  {} {} {}",
        style::styled(style::diagnostic::NOT_IDEMPOTENT, "\u{2717}"),
        result.command_name,
        style::styled(style::diagnostic::NOT_IDEMPOTENT, style::message::NOT_IDEMPOTENT));

    for diff in &result.diffs {
        match &diff.kind {
            DiffKind::ExitCode { run1, run2 } => {
                eprintln!("    {} {} {} → {}", style::dim("~"), diff.label, run1, run2);
            }
            DiffKind::FileHash { run1, run2 } => {
                eprintln!("    {} {} hash: {} → {}",
                    style::dim("~"), diff.label,
                    style::styled(style::diagnostic::NOT_IDEMPOTENT, run1),
                    style::styled(style::diagnostic::IDEMPOTENT, run2));
            }
            DiffKind::StdContent { run1, run2 } => {
                eprintln!("    {} {}", style::dim("~"), diff.label);
                format_line_diff(run1, run2);
            }
            DiffKind::ExecFailed { error } => {
                eprintln!("    {} {} {}", style::styled(style::diagnostic::NOT_IDEMPOTENT, "!"), diff.label, error);
            }
        }
    }
}

const MAX_DIFF_LINES: usize = 8;

fn format_line_diff(run1: &str, run2: &str) {
    let lines1: Vec<&str> = run1.lines().collect();
    let lines2: Vec<&str> = run2.lines().collect();
    let mut shown = 0;
    let mut total = 0;

    for i in 0..lines1.len().max(lines2.len()) {
        let l1 = lines1.get(i).copied().unwrap_or("");
        let l2 = lines2.get(i).copied().unwrap_or("");
        if l1 == l2 { continue; }
        total += 1;
        if shown >= MAX_DIFF_LINES { continue; }
        if !l1.is_empty() {
            eprintln!("      {}", style::styled(style::diagnostic::NOT_IDEMPOTENT, &format!("- {l1}")));
        }
        if !l2.is_empty() {
            eprintln!("      {}", style::styled(style::diagnostic::IDEMPOTENT, &format!("+ {l2}")));
        }
        shown += 1;
    }
    if total > MAX_DIFF_LINES {
        eprintln!("      {} ...and {} more", style::dim("~"), total - MAX_DIFF_LINES);
    }
}
