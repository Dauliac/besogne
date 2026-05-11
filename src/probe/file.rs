use super::{Probe, ProbeResult};
use std::collections::HashMap;
use std::fs;

pub struct FileProbe<'a> {
    pub path: &'a str,
}

impl<'a> Probe for FileProbe<'a> {
    fn probe(&self) -> ProbeResult {
        let metadata = match fs::metadata(self.path) {
            Ok(m) => m,
            Err(e) => {
                return ProbeResult {
                    success: false,
                    hash: String::new(),
                    variables: HashMap::new(),
                    error: Some(format!("file '{}' not found: {e}", self.path)),
                };
            }
        };

        let hash = if metadata.is_file() {
            match fs::read(self.path) {
                Ok(content) => blake3::hash(&content).to_hex().to_string(),
                Err(e) => {
                    return ProbeResult {
                        success: false,
                        hash: String::new(),
                        variables: HashMap::new(),
                        error: Some(format!("cannot read '{}': {e}", self.path)),
                    };
                }
            }
        } else {
            // Directory/socket — hash the path + mtime as fingerprint
            let mtime = metadata
                .modified()
                .map(|t| format!("{t:?}"))
                .unwrap_or_default();
            blake3::hash(format!("{}:{}", self.path, mtime).as_bytes())
                .to_hex()
                .to_string()
        };

        ProbeResult {
            success: true,
            hash,
            variables: HashMap::new(),
            error: None,
        }
    }
}
