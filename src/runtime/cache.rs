use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Context cache — stored in $XDG_CACHE_HOME/besogne/<compiler_hash>/<besogne_hash>/context.json
///
/// Two-level content-addressed layout:
/// - `<compiler_hash>`: BLAKE3 of the sealed besogne binary. Compiler update = new dir = all old caches invalidated.
/// - `<besogne_hash>`: BLAKE3 of the IR JSON. Same manifest across repos = same dir = shared cache.
///
/// GC: on save, old `<compiler_hash>` sibling dirs are removed (they belong to a previous compiler version).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ContextCache {
    pub besogne_hash: String,
    #[serde(default)]
    pub compiler_hash: String,
    /// Structural hash of the cache format — auto-derived, no manual versioning.
    /// ANY field change (add/remove/rename/retype) in ContextCache, CachedProbe,
    /// CachedCommand, or LastRun produces a different hash → cache invalidated.
    #[serde(default)]
    pub schema_hash: String,
    pub probed_at: Option<String>,
    pub warmup: HashMap<String, CachedProbe>,
    pub last_run: Option<LastRun>,
    /// Per-command cached output from the last successful run
    #[serde(default)]
    pub commands: HashMap<String, CachedCommand>,
}

/// Cached output of a single command execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedCommand {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
    pub wall_ms: u64,
    pub user_ms: u64,
    pub sys_ms: u64,
    pub max_rss_kb: u64,
    #[serde(default)]
    pub disk_read_bytes: u64,
    #[serde(default)]
    pub disk_write_bytes: u64,
    #[serde(default)]
    pub net_read_bytes: u64,
    #[serde(default)]
    pub net_write_bytes: u64,
    #[serde(default)]
    pub processes_spawned: u64,
    #[serde(default)]
    pub process_tree: Vec<crate::tracer::ProcessMetrics>,
    pub ran_at: String,
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
    /// Load cache from disk, or return empty cache.
    ///
    /// Defense-in-depth invalidation — cache is rejected if ANY of:
    /// 1. `compiler_hash` mismatch — besogne binary changed (code/schema change)
    /// 2. `schema_hash` mismatch — cache struct shape changed (new fields, type changes)
    /// 3. JSON deserialization fails — corrupt or incompatible format
    pub fn load(besogne_hash: &str) -> Self {
        let current_compiler = compiler_self_hash();
        let current_schema = cache_schema_hash();
        let fresh = || Self {
            besogne_hash: besogne_hash.to_string(),
            compiler_hash: current_compiler.clone(),
            schema_hash: current_schema.clone(),
            ..Default::default()
        };

        let path = cache_path(&current_compiler, besogne_hash);
        match std::fs::read_to_string(&path) {
            Ok(content) => {
                let loaded: Self = match serde_json::from_str(&content) {
                    Ok(c) => c,
                    Err(_) => return fresh(), // corrupt/incompatible → invalidate
                };
                // Check compiler hash
                if loaded.compiler_hash != current_compiler {
                    return fresh();
                }
                // Check schema hash — catches field additions/removals/renames
                if !loaded.schema_hash.is_empty() && loaded.schema_hash != current_schema {
                    return fresh();
                }
                Self {
                    besogne_hash: besogne_hash.to_string(),
                    schema_hash: current_schema,
                    ..loaded
                }
            }
            Err(_) => fresh(),
        }
    }

    /// Save cache to disk and GC old compiler hash directories.
    pub fn save(&self) -> Result<(), String> {
        let path = cache_path(&self.compiler_hash, &self.besogne_hash);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("cannot create cache dir: {e}"))?;
        }
        let content = serde_json::to_string_pretty(self)
            .map_err(|e| format!("cannot serialize cache: {e}"))?;
        std::fs::write(&path, content)
            .map_err(|e| format!("cannot write cache: {e}"))?;

        // GC: remove sibling compiler_hash dirs that don't match current
        gc_old_compiler_dirs(&self.compiler_hash);

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

    /// Get cached command output
    pub fn get_command(&self, command_name: &str) -> Option<&CachedCommand> {
        self.commands.get(command_name)
    }

    /// Store command output
    pub fn set_command(&mut self, command_name: String, cached: CachedCommand) {
        self.commands.insert(command_name, cached);
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

/// Compute the cache file path:
/// `$XDG_CACHE_HOME/besogne/<compiler_hash>/<besogne_hash>/context.json`
fn cache_path(compiler_hash: &str, besogne_hash: &str) -> PathBuf {
    cache_base_dir()
        .join(compiler_hash)
        .join(besogne_hash)
        .join("context.json")
}

/// Base cache directory: `$XDG_CACHE_HOME/besogne/`
fn cache_base_dir() -> PathBuf {
    let cache_dir = std::env::var("XDG_CACHE_HOME")
        .unwrap_or_else(|_| {
            let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
            format!("{home}/.cache")
        });
    Path::new(&cache_dir).join("besogne")
}

/// Structural hash of the cache format — auto-derived, no manual versioning.
///
/// Creates a canary instance with all fields populated, serializes to JSON,
/// and hashes the output. ANY structural change to ContextCache, CachedProbe,
/// CachedCommand, or LastRun (add/remove/rename/retype a field) produces
/// different JSON → different hash → all caches invalidated automatically.
fn cache_schema_hash() -> String {
    // Serialize a default instance — captures ALL fields including any added by
    // concurrent changes. Using Default means new fields are automatically included.
    let mut canary = ContextCache::default();
    canary.besogne_hash = "x".into();
    canary.compiler_hash = "x".into();
    canary.schema_hash = "x".into();
    canary.probed_at = Some("x".into());
    canary.warmup.insert("k".into(), CachedProbe {
        hash: "x".into(),
        probed_at: "x".into(),
        variables: HashMap::from([("k".into(), "v".into())]),
    });
    canary.last_run = Some(LastRun {
        input_hash: "x".into(),
        output_hash: "x".into(),
        exit_code: 0,
        ran_at: "x".into(),
        duration_ms: 0,
        skipped: false,
    });
    canary.commands.insert("k".into(), CachedCommand {
        stdout: "x".into(),
        stderr: "x".into(),
        exit_code: 0,
        wall_ms: 0,
        user_ms: 0,
        sys_ms: 0,
        max_rss_kb: 0,
        disk_read_bytes: 0,
        disk_write_bytes: 0,
        net_read_bytes: 0,
        net_write_bytes: 0,
        processes_spawned: 0,
        process_tree: vec![],
        ran_at: "x".into(),
    });
    let json = serde_json::to_string(&canary).unwrap_or_default();
    blake3::hash(json.as_bytes()).to_hex()[..8].to_string()
}

/// BLAKE3 hash of the current binary (the sealed besogne).
/// Different compiler or different manifest → different hash → cache invalidation.
pub fn compiler_self_hash() -> String {
    std::env::current_exe()
        .ok()
        .and_then(|p| std::fs::read(&p).ok())
        .map(|bytes| blake3::hash(&bytes).to_hex()[..16].to_string())
        .unwrap_or_else(|| "unknown".to_string())
}

/// Remove sibling compiler_hash directories that don't match the current one.
/// This cleans up caches from previous compiler versions.
fn gc_old_compiler_dirs(current_compiler_hash: &str) {
    let base = cache_base_dir();
    let entries = match std::fs::read_dir(&base) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        // Only remove dirs that look like hex hashes (16 chars) and don't match current
        if name_str.len() == 16
            && name_str != current_compiler_hash
            && name_str.chars().all(|c| c.is_ascii_hexdigit())
            && entry.path().is_dir()
        {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
}

fn now_iso() -> String {
    chrono::Utc::now().to_rfc3339()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Test helper: save a ContextCache directly to a given base dir (no env var dependency).
    fn save_to(cache: &ContextCache, base: &Path) -> Result<(), String> {
        let path = base
            .join("besogne")
            .join(&cache.compiler_hash)
            .join(&cache.besogne_hash)
            .join("context.json");
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("cannot create cache dir: {e}"))?;
        }
        let content = serde_json::to_string_pretty(cache)
            .map_err(|e| format!("cannot serialize cache: {e}"))?;
        std::fs::write(&path, content)
            .map_err(|e| format!("cannot write cache: {e}"))?;
        Ok(())
    }

    /// Test helper: load a ContextCache from a given base dir.
    fn load_from(base: &Path, compiler_hash: &str, besogne_hash: &str) -> ContextCache {
        let path = base
            .join("besogne")
            .join(compiler_hash)
            .join(besogne_hash)
            .join("context.json");
        match std::fs::read_to_string(&path) {
            Ok(content) => {
                let mut loaded: ContextCache = serde_json::from_str(&content)
                    .unwrap_or_else(|_| ContextCache {
                        besogne_hash: besogne_hash.to_string(),
                        compiler_hash: compiler_hash.to_string(),
                        ..Default::default()
                    });
                if loaded.compiler_hash != compiler_hash {
                    return ContextCache {
                        besogne_hash: besogne_hash.to_string(),
                        compiler_hash: compiler_hash.to_string(),
                        ..Default::default()
                    };
                }
                loaded.besogne_hash = besogne_hash.to_string();
                loaded
            }
            Err(_) => ContextCache {
                besogne_hash: besogne_hash.to_string(),
                compiler_hash: compiler_hash.to_string(),
                ..Default::default()
            },
        }
    }

    /// Test helper: run GC on a given base dir.
    fn gc_in(base: &Path, current_compiler_hash: &str) {
        let besogne_dir = base.join("besogne");
        let entries = match std::fs::read_dir(&besogne_dir) {
            Ok(e) => e,
            Err(_) => return,
        };
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            if name_str.len() == 16
                && name_str != current_compiler_hash
                && name_str.chars().all(|c| c.is_ascii_hexdigit())
                && entry.path().is_dir()
            {
                let _ = std::fs::remove_dir_all(entry.path());
            }
        }
    }

    #[test]
    fn test_cache_load_missing_returns_empty() {
        let cache = ContextCache::load("nonexistent_hash_12345");
        assert_eq!(cache.besogne_hash, "nonexistent_hash_12345");
        assert!(!cache.compiler_hash.is_empty(), "compiler_hash should be set");
        assert!(cache.warmup.is_empty());
        assert!(cache.last_run.is_none());
    }

    #[test]
    fn test_cache_save_and_load() {
        let dir = tempfile::tempdir().unwrap();
        let compiler = "aaaa000011112222";
        let besogne = "bbbb333344445555";

        let mut cache = ContextCache {
            besogne_hash: besogne.to_string(),
            compiler_hash: compiler.to_string(),
            ..Default::default()
        };
        cache.set_probe(
            "env:HOME".to_string(),
            "abc123".to_string(),
            HashMap::from([("HOME".to_string(), "/home/test".to_string())]),
        );
        cache.set_last_run("input_hash_1".to_string(), 0, 1234);
        save_to(&cache, dir.path()).unwrap();

        let loaded = load_from(dir.path(), compiler, besogne);
        assert_eq!(loaded.besogne_hash, besogne);
        assert_eq!(loaded.compiler_hash, compiler);
        assert!(loaded.warmup.contains_key("env:HOME"));
        assert_eq!(loaded.warmup["env:HOME"].hash, "abc123");
        assert!(loaded.last_run.is_some());
        assert_eq!(loaded.last_run.unwrap().exit_code, 0);
    }

    #[test]
    fn test_compiler_hash_invalidation() {
        let dir = tempfile::tempdir().unwrap();
        let compiler_a = "aaaa000011112222";
        let compiler_b = "deadbeef01234567";
        let besogne = "cccc555566667777";

        // Save with compiler A
        let mut cache = ContextCache {
            besogne_hash: besogne.to_string(),
            compiler_hash: compiler_a.to_string(),
            ..Default::default()
        };
        cache.set_last_run("input_abc".to_string(), 0, 100);
        save_to(&cache, dir.path()).unwrap();

        // Load with compiler A — should find it
        let loaded_a = load_from(dir.path(), compiler_a, besogne);
        assert!(loaded_a.can_skip("input_abc"), "same compiler should find cache");

        // Load with compiler B — should NOT find compiler A's cache
        let loaded_b = load_from(dir.path(), compiler_b, besogne);
        assert!(!loaded_b.can_skip("input_abc"), "different compiler should not see old cache");
        assert_eq!(loaded_b.compiler_hash, compiler_b);
    }

    #[test]
    fn test_gc_removes_old_compiler_dirs() {
        let dir = tempfile::tempdir().unwrap();
        let current = "aaaa000011112222";
        let old = "abcdef0123456789";
        let base = dir.path();

        // Create old compiler dir
        let old_dir = base.join("besogne").join(old);
        std::fs::create_dir_all(old_dir.join("some_besogne")).unwrap();
        std::fs::write(old_dir.join("some_besogne").join("context.json"), "{}").unwrap();
        assert!(old_dir.exists());

        // Create non-compiler dirs
        let run_dir = base.join("besogne").join("run");
        std::fs::create_dir_all(&run_dir).unwrap();
        let compiled_dir = base.join("besogne").join("compiled");
        std::fs::create_dir_all(&compiled_dir).unwrap();

        // Create current compiler dir
        let current_dir = base.join("besogne").join(current);
        std::fs::create_dir_all(&current_dir).unwrap();

        // Run GC
        gc_in(base, current);

        assert!(!old_dir.exists(), "old compiler dir should be GC'd");
        assert!(run_dir.exists(), "run/ dir should not be GC'd");
        assert!(compiled_dir.exists(), "compiled/ dir should not be GC'd");
        assert!(current_dir.exists(), "current compiler dir should survive");
    }

    #[test]
    fn test_cross_repo_sharing() {
        let dir = tempfile::tempdir().unwrap();
        let compiler = "aaaa000011112222";
        let shared_besogne = "shared_ir_hash_01";

        // "Repo A" saves
        let mut cache_a = ContextCache {
            besogne_hash: shared_besogne.to_string(),
            compiler_hash: compiler.to_string(),
            ..Default::default()
        };
        cache_a.set_probe("env:HOME".to_string(), "home_hash".to_string(), HashMap::new());
        cache_a.set_last_run("input_xyz".to_string(), 0, 200);
        save_to(&cache_a, dir.path()).unwrap();

        // "Repo B" loads the same besogne_hash — should see repo A's data
        let cache_b = load_from(dir.path(), compiler, shared_besogne);
        assert!(cache_b.can_skip("input_xyz"), "repo B should see repo A's cache");
        assert!(cache_b.warmup.contains_key("env:HOME"));
    }

    #[test]
    fn test_can_skip_with_matching_hash() {
        let mut cache = ContextCache::default();
        cache.set_last_run("hash_a".to_string(), 0, 100);
        assert!(cache.can_skip("hash_a"));
        assert!(!cache.can_skip("hash_b"));
    }

    #[test]
    fn test_can_skip_false_on_failure() {
        let mut cache = ContextCache::default();
        cache.set_last_run("hash_a".to_string(), 1, 100);
        assert!(!cache.can_skip("hash_a"));
    }

    #[test]
    fn test_can_skip_false_on_no_last_run() {
        let cache = ContextCache::default();
        assert!(!cache.can_skip("anything"));
    }
}
