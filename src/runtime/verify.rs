//! Idempotency verification and output assertion validation.

use crate::output::style;
use std::collections::HashMap;

/// Fingerprint of a single command execution — used to compare two runs.
struct CommandFingerprint {
    exit_code: i32,
    stdout_hash: String,
    stderr_hash: String,
}

impl CommandFingerprint {
    fn from_result(result: &crate::tracer::CommandResult) -> Self {
        Self {
            exit_code: result.exit_code,
            stdout_hash: blake3::hash(&result.stdout).to_hex()[..16].to_string(),
            stderr_hash: blake3::hash(&result.stderr).to_hex()[..16].to_string(),
        }
    }

    fn matches(&self, other: &Self) -> Vec<String> {
        let mut mismatches = Vec::new();
        if self.exit_code != other.exit_code {
            mismatches.push(format!(
                "{}: {} vs {}",
                style::message::VERIFY_MISMATCH_EXIT,
                self.exit_code,
                other.exit_code,
            ));
        }
        if self.stdout_hash != other.stdout_hash {
            mismatches.push(style::message::VERIFY_MISMATCH_STDOUT.to_string());
        }
        if self.stderr_hash != other.stderr_hash {
            mismatches.push(style::message::VERIFY_MISMATCH_STDERR.to_string());
        }
        mismatches
    }
}

/// Re-run a command and compare fingerprints to verify idempotency.
/// Returns None if idempotent, Some(mismatches) if not.
pub fn verify_command(
    run: &[String],
    env: &HashMap<String, String>,
    sandbox: &crate::ir::EnvSandboxResolved,
    workdir: Option<&str>,
    first_fingerprint: &crate::tracer::CommandResult,
) -> Option<Vec<String>> {
    let fp1 = CommandFingerprint::from_result(first_fingerprint);

    // Re-run
    let result2 = match crate::tracer::execute_traced(run, env, sandbox, workdir) {
        Ok(r) => r,
        Err(_) => {
            return Some(vec!["second run failed to execute".to_string()]);
        }
    };

    let fp2 = CommandFingerprint::from_result(&result2);
    let mismatches = fp1.matches(&fp2);
    if mismatches.is_empty() { None } else { Some(mismatches) }
}

/// Format a verification result line for human output.
pub fn format_verify_human(name: &str, mismatches: Option<&[String]>) -> String {
    match mismatches {
        None => {
            format!("  {} {name} {}",
                style::styled(style::verify::IDEMPOTENT, "\u{2713}"),
                style::styled(style::verify::IDEMPOTENT, style::message::IDEMPOTENT))
        }
        Some(reasons) => {
            format!("  {} {name} {} ({})",
                style::styled(style::verify::NOT_IDEMPOTENT, "\u{2717}"),
                style::styled(style::verify::NOT_IDEMPOTENT, style::message::NOT_IDEMPOTENT),
                reasons.join(", "))
        }
    }
}

fn json_path<'a>(value: &'a serde_json::Value, path: &str) -> Option<&'a serde_json::Value> {
    let mut current = value;
    for key in path.split('.') { current = current.get(key)?; }
    Some(current)
}
