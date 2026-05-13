use std::collections::HashMap;
use std::path::Path;

/// Load a config file and return a flat map of flag_name → value.
/// Supports JSON, YAML, TOML. Nested keys for subcommands:
/// ```json
/// { "verbose": true, "integration": { "timeout": "600" } }
/// ```
/// becomes: { "verbose": "1", "integration.timeout": "600" }
pub fn load_config(path: &str) -> Result<HashMap<String, String>, crate::error::BesogneError> {
    let content = std::fs::read_to_string(path)
        .map_err(|e| crate::error::BesogneError::Config(format!("cannot read config file '{path}': {e}")))?;

    let value = parse_by_extension(path, &content)?;
    let mut flat = HashMap::new();
    flatten_value(&value, "", &mut flat);
    Ok(flat)
}

fn parse_by_extension(path: &str, content: &str) -> Result<serde_json::Value, crate::error::BesogneError> {
    let ext = Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();

    match ext.as_str() {
        "json" => serde_json::from_str(content)
            .map_err(|e| crate::error::BesogneError::Config(format!("invalid JSON in '{path}': {e}"))),
        "yaml" | "yml" => serde_yaml::from_str(content)
            .map_err(|e| crate::error::BesogneError::Config(format!("invalid YAML in '{path}': {e}"))),
        "toml" => {
            let toml_val: toml::Value = content.parse()
                .map_err(|e| crate::error::BesogneError::Config(format!("invalid TOML in '{path}': {e}")))?;
            // Convert toml::Value → serde_json::Value via serialization
            let json_str = serde_json::to_string(&toml_val)
                .map_err(|e| crate::error::BesogneError::Config(format!("cannot convert TOML to JSON: {e}")))?;
            serde_json::from_str(&json_str)
                .map_err(|e| crate::error::BesogneError::Config(format!("internal error converting TOML: {e}")))
        }
        _ => Err(crate::error::BesogneError::Config(format!("unsupported config file extension '.{ext}' (use .json, .yaml, .yml, or .toml)")))
    }
}

fn flatten_value(value: &serde_json::Value, prefix: &str, out: &mut HashMap<String, String>) {
    match value {
        serde_json::Value::Object(map) => {
            for (key, val) in map {
                let full_key = if prefix.is_empty() {
                    key.clone()
                } else {
                    format!("{prefix}.{key}")
                };
                match val {
                    serde_json::Value::Object(_) => flatten_value(val, &full_key, out),
                    _ => {
                        out.insert(full_key, value_to_string(val));
                    }
                }
            }
        }
        _ => {
            if !prefix.is_empty() {
                out.insert(prefix.to_string(), value_to_string(value));
            }
        }
    }
}

fn value_to_string(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Bool(b) => if *b { "1".into() } else { "0".into() },
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::Null => String::new(),
        other => other.to_string(),
    }
}
