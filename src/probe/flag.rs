use super::{Probe, ProbeResult};
use std::collections::HashMap;

/// Flag probe — checks if a CLI flag is set and matches the expected value.
///
/// Bool flags: value=true succeeds when flag is passed, value=false when not.
/// Value flags: succeeds when flag value matches the declared value.
/// No value declared: succeeds when flag is present (any truthy value).
pub struct FlagProbe<'a> {
    pub name: &'a str,
    pub env_var: &'a str,
    pub value: Option<&'a serde_json::Value>,
    /// Current flag values from CLI/env resolution
    pub flag_env: &'a HashMap<String, String>,
}

impl<'a> Probe for FlagProbe<'a> {
    fn probe(&self) -> ProbeResult {
        let current_value = self.flag_env.get(self.env_var);

        let matches = match (self.value, current_value) {
            // value=true: flag must be present and truthy
            (Some(serde_json::Value::Bool(true)), Some(v)) => v == "true" || v == "1",
            (Some(serde_json::Value::Bool(true)), None) => false,

            // value=false: flag must be absent or falsy
            (Some(serde_json::Value::Bool(false)), Some(v)) => v == "false" || v == "0" || v.is_empty(),
            (Some(serde_json::Value::Bool(false)), None) => true,

            // value="string": flag value must match exactly
            (Some(serde_json::Value::String(expected)), Some(actual)) => actual == expected,
            (Some(serde_json::Value::String(_)), None) => false,

            // No value declared: flag must be present (any value)
            (None, Some(v)) => !v.is_empty() && v != "false" && v != "0",
            (None, None) => false,

            // Other JSON values: compare string representation
            (Some(expected), Some(actual)) => actual == &expected.to_string(),
            (Some(_), None) => false,
        };

        if matches {
            let hash_content = format!("{}={}", self.name, current_value.map(|s| s.as_str()).unwrap_or("true"));
            let hash = blake3::hash(hash_content.as_bytes()).to_hex().to_string();

            // Inject the flag value as an env var for downstream commands
            let mut variables = HashMap::new();
            if let Some(val) = current_value {
                variables.insert(self.env_var.to_string(), val.clone());
            }

            ProbeResult {
                success: true,
                hash,
                variables,
                error: None,
            }
        } else {
            ProbeResult {
                success: false,
                hash: String::new(),
                variables: HashMap::new(),
                error: Some(format!("flag '--{}' not matched (expected {:?}, got {:?})",
                    self.name,
                    self.value.map(|v| v.to_string()).unwrap_or_else(|| "present".into()),
                    current_value)),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bool_flag_true_when_set() {
        let mut flag_env = HashMap::new();
        flag_env.insert("MY_FLAG_NIX".to_string(), "true".to_string());
        let probe = FlagProbe {
            name: "nix",
            env_var: "MY_FLAG_NIX",
            value: Some(&serde_json::Value::Bool(true)),
            flag_env: &flag_env,
        };
        assert!(probe.probe().success);
    }

    #[test]
    fn test_bool_flag_true_when_not_set() {
        let flag_env = HashMap::new();
        let probe = FlagProbe {
            name: "nix",
            env_var: "MY_FLAG_NIX",
            value: Some(&serde_json::Value::Bool(true)),
            flag_env: &flag_env,
        };
        assert!(!probe.probe().success);
    }

    #[test]
    fn test_bool_flag_false_when_not_set() {
        let flag_env = HashMap::new();
        let probe = FlagProbe {
            name: "nix",
            env_var: "MY_FLAG_NIX",
            value: Some(&serde_json::Value::Bool(false)),
            flag_env: &flag_env,
        };
        assert!(probe.probe().success);
    }

    #[test]
    fn test_bool_flag_false_when_set() {
        let mut flag_env = HashMap::new();
        flag_env.insert("MY_FLAG_NIX".to_string(), "true".to_string());
        let probe = FlagProbe {
            name: "nix",
            env_var: "MY_FLAG_NIX",
            value: Some(&serde_json::Value::Bool(false)),
            flag_env: &flag_env,
        };
        assert!(!probe.probe().success);
    }

    #[test]
    fn test_value_flag_matches() {
        let mut flag_env = HashMap::new();
        flag_env.insert("MY_FLAG_TARGET".to_string(), "linux".to_string());
        let probe = FlagProbe {
            name: "target",
            env_var: "MY_FLAG_TARGET",
            value: Some(&serde_json::Value::String("linux".to_string())),
            flag_env: &flag_env,
        };
        assert!(probe.probe().success);
    }

    #[test]
    fn test_value_flag_no_match() {
        let mut flag_env = HashMap::new();
        flag_env.insert("MY_FLAG_TARGET".to_string(), "macos".to_string());
        let probe = FlagProbe {
            name: "target",
            env_var: "MY_FLAG_TARGET",
            value: Some(&serde_json::Value::String("linux".to_string())),
            flag_env: &flag_env,
        };
        assert!(!probe.probe().success);
    }

    #[test]
    fn test_presence_flag_set() {
        let mut flag_env = HashMap::new();
        flag_env.insert("MY_FLAG_VERBOSE".to_string(), "true".to_string());
        let probe = FlagProbe {
            name: "verbose",
            env_var: "MY_FLAG_VERBOSE",
            value: None,
            flag_env: &flag_env,
        };
        assert!(probe.probe().success);
    }

    #[test]
    fn test_presence_flag_not_set() {
        let flag_env = HashMap::new();
        let probe = FlagProbe {
            name: "verbose",
            env_var: "MY_FLAG_VERBOSE",
            value: None,
            flag_env: &flag_env,
        };
        assert!(!probe.probe().success);
    }
}
