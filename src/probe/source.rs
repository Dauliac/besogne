use super::{Probe, ProbeResult};
use std::collections::HashMap;

pub struct SourceProbe<'a> {
    pub format: &'a str,
    pub path: Option<&'a str>,
    pub select: Option<&'a [String]>,
    pub sealed_env: Option<&'a HashMap<String, String>>,
}

impl<'a> Probe for SourceProbe<'a> {
    fn probe(&self) -> ProbeResult {
        // If sealed at build time, use the sealed env map
        if let Some(env) = self.sealed_env {
            let filtered = filter_select(env.clone(), self.select);
            let hash = hash_env_map(&filtered);
            return ProbeResult {
                success: true,
                hash,
                variables: filtered,
                error: None,
            };
        }

        // Read content from path (std-parent mode handled at runtime)
        let content = match self.path {
            Some(p) => match std::fs::read_to_string(p) {
                Ok(c) => c,
                Err(e) => {
                    return ProbeResult {
                        success: false,
                        hash: String::new(),
                        variables: HashMap::new(),
                        error: Some(format!("source: cannot read '{p}': {e}")),
                    };
                }
            },
            None => {
                // No path and no sealed env — probe succeeds with empty vars.
                // Actual env vars will come from std parent at runtime.
                return ProbeResult {
                    success: true,
                    hash: String::new(),
                    variables: HashMap::new(),
                    error: None,
                };
            }
        };

        match parse_env_map(self.format, &content) {
            Ok(env) => {
                let filtered = filter_select(env, self.select);
                let hash = hash_env_map(&filtered);
                ProbeResult {
                    success: true,
                    hash,
                    variables: filtered,
                    error: None,
                }
            }
            Err(e) => ProbeResult {
                success: false,
                hash: String::new(),
                variables: HashMap::new(),
                error: Some(format!("source: {e}")),
            },
        }
    }
}

/// Parse content into an env var map based on format
pub fn parse_env_map(format: &str, content: &str) -> Result<HashMap<String, String>, crate::error::BesogneError> {
    match format {
        "json" => parse_json(content),
        "dotenv" => parse_dotenv(content),
        "shell" => parse_shell_export(content),
        _ => Err(crate::error::BesogneError::Source(format!("unknown source format: '{format}'"))),
    }
}

/// Parse flat JSON object: {"KEY": "value", ...}
fn parse_json(content: &str) -> Result<HashMap<String, String>, crate::error::BesogneError> {
    let value: serde_json::Value =
        serde_json::from_str(content).map_err(|e| crate::error::BesogneError::Source(format!("invalid JSON: {e}")))?;

    let obj = value
        .as_object()
        .ok_or_else(|| crate::error::BesogneError::Source("JSON must be a flat object".to_string()))?;

    let mut env = HashMap::new();
    for (key, val) in obj {
        match val {
            serde_json::Value::String(s) => {
                env.insert(key.clone(), s.clone());
            }
            serde_json::Value::Null => {
                // null = unset, skip
            }
            other => {
                env.insert(key.clone(), other.to_string());
            }
        }
    }
    Ok(env)
}

/// Parse dotenv format: KEY=value lines, # comments, blank lines
fn parse_dotenv(content: &str) -> Result<HashMap<String, String>, crate::error::BesogneError> {
    let mut env = HashMap::new();
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        // Strip optional "export " prefix
        let line = line.strip_prefix("export ").unwrap_or(line);
        if let Some((key, val)) = line.split_once('=') {
            let key = key.trim();
            let val = val.trim();
            // Strip surrounding quotes
            let val = val
                .strip_prefix('"')
                .and_then(|v| v.strip_suffix('"'))
                .or_else(|| val.strip_prefix('\'').and_then(|v| v.strip_suffix('\'')))
                .unwrap_or(val);
            env.insert(key.to_string(), val.to_string());
        }
    }
    Ok(env)
}

/// Parse shell export format from `nix print-dev-env` and similar.
/// Only extracts lines matching: `KEY='literal'` or `export KEY='literal'`.
/// Single-quoted values are taken literally (no interpolation in shell).
/// Also accepts simple unquoted literals (no $, no subshell, no semicolons).
fn parse_shell_export(content: &str) -> Result<HashMap<String, String>, crate::error::BesogneError> {
    let mut env = HashMap::new();
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        // Strip optional "export " prefix
        let assignment = line.strip_prefix("export ").unwrap_or(line);
        // Must start with a valid variable name followed by =
        let Some((key, val)) = assignment.split_once('=') else { continue };
        let key = key.trim();
        if key.is_empty()
            || !key.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
            || key.as_bytes()[0].is_ascii_digit()
        {
            continue;
        }
        // Only accept single-quoted values (literal, no interpolation)
        // This is what `nix print-dev-env` uses for actual variable exports.
        let val = val.trim();
        let parsed_val = if let Some(inner) = val.strip_prefix('\'').and_then(|v| v.strip_suffix('\'')) {
            inner.to_string()
        } else {
            continue;
        };
        env.insert(key.to_string(), parsed_val);
    }
    Ok(env)
}

/// Filter env map to only include selected keys
fn filter_select(
    env: HashMap<String, String>,
    select: Option<&[String]>,
) -> HashMap<String, String> {
    match select {
        Some(keys) => env
            .into_iter()
            .filter(|(k, _)| keys.contains(k))
            .collect(),
        None => env,
    }
}

/// Hash env map deterministically for cache key
fn hash_env_map(env: &HashMap<String, String>) -> String {
    let mut pairs: Vec<(&String, &String)> = env.iter().collect();
    pairs.sort_by_key(|(k, _)| *k);
    let content: String = pairs
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join("\n");
    blake3::hash(content.as_bytes()).to_hex().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_json() {
        let env = parse_json(r#"{"GOPATH": "/home/user/go", "PATH": "/usr/bin"}"#).unwrap();
        assert_eq!(env.get("GOPATH").unwrap(), "/home/user/go");
        assert_eq!(env.get("PATH").unwrap(), "/usr/bin");
    }

    #[test]
    fn test_parse_json_null_skipped() {
        let env = parse_json(r#"{"KEEP": "yes", "REMOVE": null}"#).unwrap();
        assert_eq!(env.len(), 1);
        assert!(env.contains_key("KEEP"));
    }

    #[test]
    fn test_parse_dotenv() {
        let content = r#"
# comment
GOPATH=/home/user/go
SECRET="my secret"
SINGLE='quoted'
export EXPORTED=val
"#;
        let env = parse_dotenv(content).unwrap();
        assert_eq!(env.get("GOPATH").unwrap(), "/home/user/go");
        assert_eq!(env.get("SECRET").unwrap(), "my secret");
        assert_eq!(env.get("SINGLE").unwrap(), "quoted");
        assert_eq!(env.get("EXPORTED").unwrap(), "val");
    }

    #[test]
    fn test_select_filter() {
        let mut env = HashMap::new();
        env.insert("KEEP".into(), "yes".into());
        env.insert("DROP".into(), "no".into());
        let filtered = filter_select(env, Some(&["KEEP".to_string()]));
        assert_eq!(filtered.len(), 1);
        assert!(filtered.contains_key("KEEP"));
    }

    #[test]
    fn test_select_none_keeps_all() {
        let mut env = HashMap::new();
        env.insert("A".into(), "1".into());
        env.insert("B".into(), "2".into());
        let filtered = filter_select(env, None);
        assert_eq!(filtered.len(), 2);
    }

    #[test]
    fn test_hash_deterministic() {
        let mut env = HashMap::new();
        env.insert("B".into(), "2".into());
        env.insert("A".into(), "1".into());
        let h1 = hash_env_map(&env);
        let h2 = hash_env_map(&env);
        assert_eq!(h1, h2);
    }

    #[test]
    fn test_probe_with_path() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".env");
        std::fs::write(&path, "FOO=bar\nBAZ=qux").unwrap();

        let probe = SourceProbe {
            format: "dotenv",
            path: Some(path.to_str().unwrap()),
            select: None,
            sealed_env: None,
        };
        let result = probe.probe();
        assert!(result.success);
        assert_eq!(result.variables.get("FOO").unwrap(), "bar");
        assert_eq!(result.variables.get("BAZ").unwrap(), "qux");
    }

    #[test]
    fn test_probe_missing_file() {
        let probe = SourceProbe {
            format: "dotenv",
            path: Some("/nonexistent/.env.source.test"),
            select: None,
            sealed_env: None,
        };
        let result = probe.probe();
        assert!(!result.success);
        assert!(result.error.unwrap().contains("cannot read"));
    }

    #[test]
    fn test_probe_no_path_succeeds_empty() {
        let probe = SourceProbe {
            format: "json",
            path: None,
            select: None,
            sealed_env: None,
        };
        let result = probe.probe();
        assert!(result.success);
        assert!(result.variables.is_empty());
    }

    #[test]
    fn test_probe_sealed_env() {
        let mut sealed = HashMap::new();
        sealed.insert("SEALED_VAR".into(), "sealed_val".into());
        let probe = SourceProbe {
            format: "json",
            path: None,
            select: None,
            sealed_env: Some(&sealed),
        };
        let result = probe.probe();
        assert!(result.success);
        assert_eq!(result.variables.get("SEALED_VAR").unwrap(), "sealed_val");
    }
}
