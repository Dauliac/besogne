use super::{Probe, ProbeResult};
use std::collections::HashMap;

pub struct EnvProbe<'a> {
    pub name: &'a str,
    pub value: Option<&'a str>,
    pub secret: bool,
}

impl<'a> Probe for EnvProbe<'a> {
    fn probe(&self) -> ProbeResult {
        let resolved_value = match self.value {
            // value field set → use it (computed env var)
            Some(v) => v.to_string(),
            // No value → read from shell
            None => match std::env::var(self.name) {
                Ok(v) => v,
                Err(_) => {
                    return ProbeResult {
                        success: false,
                        hash: String::new(),
                        variables: HashMap::new(),
                        error: Some(format!("env var '{}' is not set", self.name)),
                    };
                }
            },
        };

        let hash = blake3::hash(resolved_value.as_bytes()).to_hex().to_string();

        let mut variables = HashMap::new();
        if !self.secret {
            variables.insert(self.name.to_string(), resolved_value.clone());
        }

        // Set the env var for downstream commands
        std::env::set_var(self.name, &resolved_value);

        ProbeResult {
            success: true,
            hash,
            variables,
            error: None,
        }
    }
}
