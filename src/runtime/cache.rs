use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Context cache — stored in $XDG_CACHE_HOME/besogne/<besogne_hash>/
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ContextCache {
    pub besogne_hash: String,
    pub probed_at: Option<String>,
    pub warmup: HashMap<String, CachedProbe>,
    pub last_run: Option<LastRun>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedProbe {
    pub hash: String,
    pub probed_at: String,
    pub variables: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LastRun {
    pub input_hash: String,
    pub output_hash: String,
    pub exit_code: i32,
    pub ran_at: String,
    pub duration_ms: u64,
    pub skipped: bool,
}

impl ContextCache {
    /// Load cache from disk, or return empty cache
    pub fn load(besogne_hash: &str) -> Self {
        let path = cache_path(besogne_hash);
        match std::fs::read_to_string(&path) {
            Ok(content) => serde_json::from_str(&content).unwrap_or_else(|_| Self {
                besogne_hash: besogne_hash.to_string(),
                ..Default::default()
            }),
            Err(_) => Self {
                besogne_hash: besogne_hash.to_string(),
                ..Default::default()
            },
        }
    }

    /// Save cache to disk
    pub fn save(&self) -> Result<(), String> {
        let path = cache_path(&self.besogne_hash);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("cannot create cache dir: {e}"))?;
        }
        let content = serde_json::to_string_pretty(self)
            .map_err(|e| format!("cannot serialize cache: {e}"))?;
        std::fs::write(&path, content)
            .map_err(|e| format!("cannot write cache: {e}"))?;
        Ok(())
    }

    /// Check if we can skip based on input hash + output existence
    pub fn can_skip(&self, current_input_hash: &str) -> bool {
        if let Some(last) = &self.last_run {
            last.input_hash == current_input_hash && last.exit_code == 0
        } else {
            false
        }
    }

    /// Get cached probe result for an input
    pub fn get_probe(&self, input_id: &str) -> Option<&CachedProbe> {
        self.warmup.get(input_id)
    }

    /// Store probe result
    pub fn set_probe(&mut self, input_id: String, hash: String, variables: HashMap<String, String>) {
        self.warmup.insert(
            input_id,
            CachedProbe {
                hash,
                probed_at: now_iso(),
                variables,
            },
        );
    }

    /// Store last run result
    pub fn set_last_run(&mut self, input_hash: String, exit_code: i32, duration_ms: u64) {
        self.probed_at = Some(now_iso());
        self.last_run = Some(LastRun {
            input_hash,
            output_hash: String::new(), // TODO: compute from command outputs
            exit_code,
            ran_at: now_iso(),
            duration_ms,
            skipped: false,
        });
    }
}

/// Compute the cache file path
fn cache_path(besogne_hash: &str) -> PathBuf {
    let cache_dir = std::env::var("XDG_CACHE_HOME")
        .unwrap_or_else(|_| {
            let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
            format!("{home}/.cache")
        });
    Path::new(&cache_dir)
        .join("besogne")
        .join(besogne_hash)
        .join("context.json")
}

fn now_iso() -> String {
    chrono::Utc::now().to_rfc3339()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cache_load_missing_returns_empty() {
        let cache = ContextCache::load("nonexistent_hash_12345");
        assert_eq!(cache.besogne_hash, "nonexistent_hash_12345");
        assert!(cache.warmup.is_empty());
        assert!(cache.last_run.is_none());
    }

    #[test]
    fn test_cache_save_and_load() {
        let hash = format!("test_{}", std::process::id());
        let mut cache = ContextCache::load(&hash);
        cache.set_probe(
            "env:HOME".to_string(),
            "abc123".to_string(),
            HashMap::from([("HOME".to_string(), "/home/test".to_string())]),
        );
        cache.set_last_run("input_hash_1".to_string(), 0, 1234);

        cache.save().unwrap();

        let loaded = ContextCache::load(&hash);
        assert_eq!(loaded.besogne_hash, hash);
        assert!(loaded.warmup.contains_key("env:HOME"));
        assert_eq!(loaded.warmup["env:HOME"].hash, "abc123");
        assert!(loaded.last_run.is_some());
        assert_eq!(loaded.last_run.unwrap().exit_code, 0);

        // Cleanup
        let path = cache_path(&hash);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn test_can_skip_with_matching_hash() {
        let mut cache = ContextCache::default();
        cache.set_last_run("hash_a".to_string(), 0, 100);
        assert!(cache.can_skip("hash_a"));
        assert!(!cache.can_skip("hash_b")); // different hash
    }

    #[test]
    fn test_can_skip_false_on_failure() {
        let mut cache = ContextCache::default();
        cache.set_last_run("hash_a".to_string(), 1, 100); // exit code 1
        assert!(!cache.can_skip("hash_a")); // can't skip a failed run
    }

    #[test]
    fn test_can_skip_false_on_no_last_run() {
        let cache = ContextCache::default();
        assert!(!cache.can_skip("anything"));
    }
}
