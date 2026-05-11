use std::collections::HashMap;
use std::path::Path;
use std::process::Command;

/// Evaluate a Nickel file to JSON using `nickel export`.
/// Returns the JSON string output.
pub fn eval_file(path: &Path) -> Result<String, String> {
    let output = Command::new("nickel")
        .args(["export", path.to_str().unwrap_or("")])
        .output()
        .map_err(|e| {
            format!(
                "cannot run `nickel export {}`: {e}\nhint: is `nickel` in PATH? (add to devShell)",
                path.display()
            )
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "nickel export {} failed (exit {}):\n{stderr}",
            path.display(),
            output.status.code().unwrap_or(-1)
        ));
    }

    String::from_utf8(output.stdout)
        .map_err(|e| format!("nickel output is not valid UTF-8: {e}"))
}

/// Evaluate a Nickel expression string to JSON.
/// Used for inline plugin expansion: `nickel export --field produces <<< '(import "plugin.ncl") & { params = { ... } }'`
pub fn eval_expr(expr: &str) -> Result<String, String> {
    let output = Command::new("nickel")
        .args(["export"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .and_then(|mut child| {
            use std::io::Write;
            if let Some(ref mut stdin) = child.stdin {
                stdin.write_all(expr.as_bytes())?;
            }
            child.wait_with_output()
        })
        .map_err(|e| format!("cannot run `nickel export`: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("nickel eval failed (exit {}):\n{stderr}", output.status.code().unwrap_or(-1)));
    }

    String::from_utf8(output.stdout)
        .map_err(|e| format!("nickel output is not valid UTF-8: {e}"))
}

/// Evaluate a plugin's `produces` function with the given params.
/// The plugin .ncl file must export `{ produces : params -> Array Input }`.
/// We construct: `(import "path") & { __params = { ... } } |> .produces __params`
/// and evaluate it to get the JSON array of native inputs.
pub fn eval_plugin(
    plugin_path: &Path,
    params: &HashMap<String, serde_json::Value>,
) -> Result<Vec<serde_json::Value>, String> {
    let params_json = serde_json::to_string(params)
        .map_err(|e| format!("cannot serialize plugin params: {e}"))?;

    // Nickel expression that imports the plugin and calls produces with params
    let expr = format!(
        r#"let plugin = import "{}" in
let params = std.deserialize 'Json "{}" in
plugin.produces params"#,
        plugin_path.display().to_string().replace('\\', "/").replace('"', "\\\""),
        params_json.replace('\\', "\\\\").replace('"', "\\\""),
    );

    let json_str = eval_expr(&expr)?;
    let value: serde_json::Value = serde_json::from_str(&json_str)
        .map_err(|e| format!("plugin {} did not produce valid JSON: {e}", plugin_path.display()))?;

    match value {
        serde_json::Value::Array(arr) => Ok(arr),
        _ => Err(format!(
            "plugin {} `produces` must return an array, got: {}",
            plugin_path.display(),
            value
        )),
    }
}

/// Check if `nickel` is available in PATH.
pub fn is_available() -> bool {
    Command::new("nickel")
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}
