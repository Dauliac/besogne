//! Output assertion validation for commands.

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

fn json_path<'a>(value: &'a serde_json::Value, path: &str) -> Option<&'a serde_json::Value> {
    let mut current = value;
    for key in path.split('.') { current = current.get(key)?; }
    Some(current)
}
