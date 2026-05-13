use super::{Probe, ProbeResult};
use crate::ir::types::BinarySourceResolved;
use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Command;

// ─── Build-time resolution ──────────────────────────────────────

/// Result of resolving a binary at build time
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct ResolvedBinary {
    pub name: String,
    pub path: PathBuf,
    pub canonical_path: PathBuf,
    pub source: BinarySourceResolved,
    pub version: Option<String>,
    pub hash: String,
}

/// Resolve a binary at build time: find path, detect source, hash, extract version.
///
/// `probe_version_flag`: if true, allow --version probing for System binaries
/// (enabled when user sets `version` field in manifest).
#[allow(dead_code)]
pub fn resolve_binary(
    name: &str,
    explicit_path: Option<&str>,
    probe_version_flag: bool,
) -> Result<ResolvedBinary, crate::error::BesogneError> {
    resolve_binary_with_cache(name, explicit_path, probe_version_flag, None)
}

/// Resolve with an optional shared hash cache (keyed by canonical path).
/// Multiple binaries pointing to the same file (e.g., coreutils) are hashed once.
pub fn resolve_binary_with_cache(
    name: &str,
    explicit_path: Option<&str>,
    probe_version_flag: bool,
    hash_cache: Option<&std::sync::Mutex<std::collections::HashMap<PathBuf, String>>>,
) -> Result<ResolvedBinary, crate::error::BesogneError> {
    // 1. Resolve raw path
    let raw_path = match explicit_path {
        Some(p) => {
            let pb = PathBuf::from(p);
            if !pb.exists() {
                return Err(crate::error::BesogneError::Probe(format!(
                    "binary '{name}' not found at {p}\n  hint: check that the path is correct"
                )));
            }
            pb
        }
        None => resolve_via_path(name).ok_or_else(|| {
            crate::error::BesogneError::Probe(format!(
                "binary '{name}' not found in PATH\n  \
                 hint: add it to your PATH or set an explicit \"path\" in the manifest"
            ))
        })?,
    };

    // 2. Resolve symlinks to find true location (important for Nix profiles, mise)
    let canonical = std::fs::canonicalize(&raw_path).unwrap_or_else(|_| raw_path.clone());

    // 3. Detect source from canonical path
    let source = detect_source(&canonical);

    // 4. Extract version based on source
    let version = extract_version(&source, &canonical, probe_version_flag);

    // 5. Hash the binary (deduplicated by canonical path)
    let hash = if let Some(cache) = hash_cache {
        let guard = cache.lock().unwrap();
        if let Some(cached) = guard.get(&canonical) {
            cached.clone()
        } else {
            drop(guard);
            let h = hash_binary(&canonical, name)?;
            cache.lock().unwrap().insert(canonical.clone(), h.clone());
            h
        }
    } else {
        hash_binary(&canonical, name)?
    };

    Ok(ResolvedBinary {
        name: name.to_string(),
        path: raw_path,
        canonical_path: canonical,
        source,
        version,
        hash,
    })
}

fn hash_binary(canonical: &PathBuf, name: &str) -> Result<String, crate::error::BesogneError> {
    match std::fs::read(canonical) {
        Ok(content) => Ok(blake3::hash(&content).to_hex().to_string()),
        Err(e) => Err(crate::error::BesogneError::Probe(format!("cannot read binary '{name}': {e}"))),
    }
}

// ─── Source detection ───────────────────────────────────────────

/// Detect the source of a binary from its canonical (symlink-resolved) path.
fn detect_source(canonical_path: &PathBuf) -> BinarySourceResolved {
    let path_str = canonical_path.to_string_lossy();

    if path_str.starts_with("/nix/store/") {
        return detect_nix(&path_str);
    }

    if let Some(mise) = detect_mise(&path_str) {
        return mise;
    }

    BinarySourceResolved::System
}

/// Detect a Nix binary: query derivation metadata, fall back to store path parsing.
fn detect_nix(path_str: &str) -> BinarySourceResolved {
    let store_entry = path_str
        .strip_prefix("/nix/store/")
        .and_then(|rest| rest.split('/').next())
        .unwrap_or("");
    let store_path = format!("/nix/store/{store_entry}");

    // Primary: query real derivation metadata via nix-store + nix derivation show
    if let Some(meta) = query_nix_derivation_metadata(&store_path) {
        return BinarySourceResolved::Nix {
            store_path,
            pname: Some(meta.pname),
        };
    }

    // Fallback: parse store path name heuristically
    let (pname, _) = parse_nix_store_name(store_entry);
    BinarySourceResolved::Nix { store_path, pname }
}

/// Metadata extracted from a Nix derivation
#[derive(Debug, Clone)]
struct NixDerivationMeta {
    pname: String,
    version: String,
}

/// Query Nix derivation metadata for a store path.
///
/// 1. `nix-store --query --deriver <store_path>` → drv path
/// 2. `nix derivation show <drv_path>` → JSON with pname/version
///
/// Handles both `__json` structured env (modern nixpkgs) and direct env fields.
fn query_nix_derivation_metadata(store_path: &str) -> Option<NixDerivationMeta> {
    // Step 1: get deriver (.drv path)
    let deriver = Command::new("nix-store")
        .args(["--query", "--deriver", store_path])
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    if !deriver.status.success() {
        return None;
    }
    let drv_path = String::from_utf8_lossy(&deriver.stdout).trim().to_string();
    if drv_path.is_empty() || drv_path == "unknown-deriver" {
        return None;
    }

    // Step 2: show derivation as JSON
    let drv_show = Command::new("nix")
        .args(["derivation", "show", &drv_path])
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    if !drv_show.status.success() {
        return None;
    }

    let json: serde_json::Value = serde_json::from_slice(&drv_show.stdout).ok()?;
    let drv_data = json.as_object()?.values().next()?;
    let env = drv_data.get("env")?.as_object()?;

    // Modern nixpkgs: metadata is inside a __json field
    if let Some(json_str) = env.get("__json").and_then(|v| v.as_str()) {
        let parsed: serde_json::Value = serde_json::from_str(json_str).ok()?;
        let pname = parsed.get("pname")?.as_str()?.to_string();
        let version = parsed.get("version")?.as_str()?.to_string();
        if !pname.is_empty() && !version.is_empty() {
            return Some(NixDerivationMeta { pname, version });
        }
    }

    // Legacy: pname/version directly in env
    let pname = env.get("pname").and_then(|v| v.as_str())?.to_string();
    let version = env.get("version").and_then(|v| v.as_str())?.to_string();
    if !pname.is_empty() && !version.is_empty() {
        return Some(NixDerivationMeta { pname, version });
    }

    None
}

/// Fallback: parse a Nix store entry heuristically.
/// "<hash>-<pname>-<version>" → (pname, version)
///
/// Version starts with a digit. Uses leftmost match to avoid including
/// non-version suffixes (e.g. "-npm" in "nodejs-slim-24.14.1-npm").
fn parse_nix_store_name(store_entry: &str) -> (Option<String>, Option<String>) {
    let after_hash = match store_entry.find('-') {
        Some(idx) => &store_entry[idx + 1..],
        None => return (None, None),
    };

    // Find the first '-' followed by a digit — that's where version starts
    let version_regex = regex::Regex::new(r"-(\d+\.\d+[^\-]*)").ok();
    if let Some(re) = &version_regex {
        if let Some(m) = re.find(after_hash) {
            let pname = &after_hash[..m.start()];
            let version = &after_hash[m.start() + 1..m.end()]; // skip leading '-'
            if !pname.is_empty() {
                return (Some(pname.to_string()), Some(version.to_string()));
            }
        }
    }

    (Some(after_hash.to_string()), None)
}

/// Detect mise-managed binary: .../installs/<tool>/<version>/...
fn detect_mise(path_str: &str) -> Option<BinarySourceResolved> {
    // Look for the mise installs pattern anywhere in the path
    // Handles both default (~/.local/share/mise/installs/) and custom MISE_DATA_DIR
    let idx = path_str.find("/installs/")?;
    let after = &path_str[idx + "/installs/".len()..];
    let mut parts = after.splitn(3, '/');
    let tool = parts.next().filter(|t| !t.is_empty())?;
    // Must have a version segment after tool
    let _version = parts.next().filter(|v| !v.is_empty())?;

    Some(BinarySourceResolved::Mise {
        tool: tool.to_string(),
    })
}

// ─── Version extraction ─────────────────────────────────────────

/// Extract version based on source type. No execution for Nix/Mise.
/// System binaries only probed if `probe_version_flag` is true.
fn extract_version(
    source: &BinarySourceResolved,
    canonical_path: &PathBuf,
    _probe_version_flag: bool,
) -> Option<String> {
    match source {
        BinarySourceResolved::Nix { .. } => nix_version(canonical_path),
        BinarySourceResolved::Mise { .. } => mise_version(canonical_path),
        BinarySourceResolved::System => {
            // Always probe version for System binaries — UX display only.
            // The binary BLAKE3 hash is the real cache key, so imperfect
            // version detection is fine.
            probe_version_from_binary(canonical_path)
        }
    }
}

/// Extract version from Nix derivation metadata, falling back to store path parsing.
fn nix_version(canonical_path: &PathBuf) -> Option<String> {
    let path_str = canonical_path.to_string_lossy();
    let store_entry = path_str
        .strip_prefix("/nix/store/")
        .and_then(|rest| rest.split('/').next())?;
    let store_path = format!("/nix/store/{store_entry}");

    // Primary: derivation metadata
    if let Some(meta) = query_nix_derivation_metadata(&store_path) {
        return Some(meta.version);
    }

    // Fallback: parse store path
    let (_, version) = parse_nix_store_name(store_entry);
    version
}

/// Extract version from mise install path (no execution needed)
fn mise_version(canonical_path: &PathBuf) -> Option<String> {
    let path_str = canonical_path.to_string_lossy();
    let idx = path_str.find("/installs/")?;
    let after = &path_str[idx + "/installs/".len()..];
    let mut parts = after.splitn(3, '/');
    let _tool = parts.next()?;
    let version = parts.next().filter(|v| !v.is_empty())?;
    Some(version.to_string())
}

/// Probe version by executing binary with --version/-v/version flags.
///
/// Uses a multi-strategy approach to extract version from diverse output formats:
/// - GNU coreutils: `tee (GNU coreutils) 9.4`
/// - Standard: `bat 0.26.1`, `go version go1.22.0 linux/amd64`
/// - Semver with pre-release: `node v22.11.0`, `rust 1.82.0-nightly`
/// - Multi-line: version on first meaningful line
///
/// This is for UX display only — the binary BLAKE3 hash is the real cache key.
fn probe_version_from_binary(binary_path: &PathBuf) -> Option<String> {
    // Full semver + pre-release + build metadata
    // Matches: 9.4, 1.22.0, 0.26.1, 1.82.0-nightly, 1.2.3+build.42
    let version_regex = regex::Regex::new(
        r"[vV]?(\d+\.\d+(?:\.\d+)?(?:-[a-zA-Z0-9._]+)?(?:\+[a-zA-Z0-9._]+)?)"
    ).ok()?;

    for flag in &["--version", "-v", "version"] {
        let output = std::process::Command::new(binary_path)
            .arg(flag)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .output()
            .ok()?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        let _stderr = String::from_utf8_lossy(&output.stderr);

        // Try first line of stdout first (most tools put version there)
        // then fall back to stderr, then full output
        let first_line = stdout.lines().next().unwrap_or("");
        let candidates = [first_line, &stdout, &String::from_utf8_lossy(&output.stderr)];

        for text in &candidates {
            if let Some(ver) = extract_version_from_text(text, &version_regex) {
                return Some(ver);
            }
        }
    }
    None
}

/// Extract a version string from text output, handling common formats.
fn extract_version_from_text(text: &str, re: &regex::Regex) -> Option<String> {
    if text.trim().is_empty() {
        return None;
    }

    // Find the first version-like match
    let caps = re.captures(text)?;
    let version = caps.get(1)?.as_str().to_string();

    // Sanity: reject implausible matches (e.g., dates like 2023.01.01 from copyright lines)
    // A version should not start with 19xx or 20xx followed by month-like numbers
    if version.starts_with("19") || version.starts_with("20") {
        let parts: Vec<&str> = version.split('.').collect();
        if parts.len() >= 2 {
            if let Ok(second) = parts[1].parse::<u32>() {
                if second >= 1 && second <= 12 {
                    // Looks like a date (2023.01, 2024.12), skip and try next match
                    let remaining = &text[caps.get(0)?.end()..];
                    return extract_version_from_text(remaining, re);
                }
            }
        }
    }

    Some(version)
}

// ─── PATH resolution ────────────────────────────────────────────

fn resolve_via_path(name: &str) -> Option<PathBuf> {
    let path_var = std::env::var("PATH").ok()?;
    for dir in path_var.split(':') {
        let candidate = PathBuf::from(dir).join(name);
        if candidate.exists() {
            return Some(candidate);
        }
    }
    None
}

// ─── Runtime probe ──────────────────────────────────────────────

/// Runtime binary probe — lightweight re-verification of a (typically build-resolved) binary.
///
/// If build-time data is available (resolved_path, source, hash), uses it for
/// efficient verification. Otherwise falls back to PATH resolution.
pub struct BinaryProbe<'a> {
    pub name: &'a str,
    /// Explicit path from manifest
    pub path: Option<&'a str>,
    /// Build-time resolved source
    pub source: Option<&'a BinarySourceResolved>,
    /// Build-time resolved canonical path
    pub resolved_path: Option<&'a str>,
    /// Build-time resolved version
    pub resolved_version: Option<&'a str>,
    /// Build-time BLAKE3 hash
    #[allow(dead_code)]
    pub binary_hash: Option<&'a str>,
}

impl<'a> Probe for BinaryProbe<'a> {
    fn probe(&self) -> ProbeResult {
        // Use build-time resolved path if available, otherwise resolve now
        let resolved_path = match self.resolved_path {
            Some(p) => PathBuf::from(p),
            None => {
                // Fallback: resolve via explicit path or PATH
                match self.path {
                    Some(p) => PathBuf::from(p),
                    None => match resolve_via_path(self.name) {
                        Some(p) => p,
                        None => {
                            return ProbeResult {
                                success: false,
                                hash: String::new(),
                                variables: HashMap::new(),
                                error: Some(format!(
                                    "binary '{}' not found in PATH",
                                    self.name
                                )),
                            };
                        }
                    },
                }
            }
        };

        if !resolved_path.exists() {
            let source_hint = match self.source {
                Some(BinarySourceResolved::Nix { store_path, .. }) => {
                    format!(" (Nix store path: {store_path} — may have been garbage collected)")
                }
                Some(BinarySourceResolved::Mise { tool }) => {
                    format!(" (mise tool: {tool} — run 'mise install' to restore)")
                }
                _ => String::new(),
            };
            return ProbeResult {
                success: false,
                hash: String::new(),
                variables: HashMap::new(),
                error: Some(format!(
                    "binary '{}' not found at {}{}",
                    self.name,
                    resolved_path.display(),
                    source_hint,
                )),
            };
        }

        // Hash the binary
        let hash = match std::fs::read(&resolved_path) {
            Ok(content) => blake3::hash(&content).to_hex().to_string(),
            Err(e) => {
                return ProbeResult {
                    success: false,
                    hash: String::new(),
                    variables: HashMap::new(),
                    error: Some(format!("cannot read binary: {e}")),
                };
            }
        };

        // Use build-time version (already extracted safely), no --version probing
        let version = self.resolved_version.map(|v| v.to_string());

        // Compute parent dir (go up from bin/ to package root)
        let dir = resolved_path
            .parent()
            .and_then(|p| p.parent())
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default();

        let upper_name = self.name.to_uppercase().replace('-', "_");
        let mut variables = HashMap::new();
        variables.insert(
            upper_name.clone(),
            resolved_path.to_string_lossy().to_string(),
        );
        if let Some(v) = &version {
            variables.insert(format!("{upper_name}_VERSION"), v.clone());
        }
        variables.insert(format!("{upper_name}_DIR"), dir);

        // Add source-specific variables
        match self.source {
            Some(BinarySourceResolved::Nix { store_path, pname }) => {
                variables.insert(
                    format!("{upper_name}_NIX_STORE"),
                    store_path.clone(),
                );
                if let Some(pname) = pname {
                    variables.insert(format!("{upper_name}_NIX_PNAME"), pname.clone());
                }
            }
            Some(BinarySourceResolved::Mise { tool }) => {
                variables.insert(format!("{upper_name}_MISE_TOOL"), tool.clone());
            }
            _ => {}
        }

        ProbeResult {
            success: true,
            hash,
            variables,
            error: None,
        }
    }
}

// ─── Tests ──────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_nix_store_name_with_version() {
        let (pname, version) =
            parse_nix_store_name("abc123def-nodejs-22.11.0");
        assert_eq!(pname.as_deref(), Some("nodejs"));
        assert_eq!(version.as_deref(), Some("22.11.0"));
    }

    #[test]
    fn test_parse_nix_store_name_hyphenated_pname() {
        let (pname, version) =
            parse_nix_store_name("abc123def-xorg-libX11-1.8.1");
        assert_eq!(pname.as_deref(), Some("xorg-libX11"));
        assert_eq!(version.as_deref(), Some("1.8.1"));
    }

    #[test]
    fn test_parse_nix_store_name_no_version() {
        let (pname, version) = parse_nix_store_name("abc123def-coreutils");
        assert_eq!(pname.as_deref(), Some("coreutils"));
        assert_eq!(version, None);
    }

    #[test]
    fn test_parse_nix_store_name_go() {
        let (pname, version) = parse_nix_store_name("abc123def-go-1.22.0");
        assert_eq!(pname.as_deref(), Some("go"));
        assert_eq!(version.as_deref(), Some("1.22.0"));
    }

    #[test]
    fn test_parse_nix_store_name_with_output_suffix() {
        // "nodejs-slim-24.14.1-npm" — the "-npm" is an output name, not version
        // The fallback regex picks the first \d+\.\d+ match
        let (pname, version) =
            parse_nix_store_name("abc123def-nodejs-slim-24.14.1-npm");
        assert_eq!(pname.as_deref(), Some("nodejs-slim"));
        assert_eq!(version.as_deref(), Some("24.14.1"));
    }

    #[test]
    fn test_detect_source_nix() {
        let path = PathBuf::from("/nix/store/abc123-nodejs-22.11.0/bin/node");
        let source = detect_source(&path);
        match source {
            BinarySourceResolved::Nix { store_path, pname } => {
                assert_eq!(store_path, "/nix/store/abc123-nodejs-22.11.0");
                assert_eq!(pname.as_deref(), Some("nodejs"));
            }
            _ => panic!("expected Nix source"),
        }
    }

    #[test]
    fn test_detect_source_mise() {
        let path = PathBuf::from(
            "/home/user/.local/share/mise/installs/node/22.22.2/bin/node",
        );
        let source = detect_source(&path);
        match source {
            BinarySourceResolved::Mise { tool } => {
                assert_eq!(tool, "node");
            }
            _ => panic!("expected Mise source"),
        }
    }

    #[test]
    fn test_detect_source_system() {
        let path = PathBuf::from("/usr/bin/ls");
        let source = detect_source(&path);
        assert!(matches!(source, BinarySourceResolved::System));
    }

    #[test]
    fn test_mise_version_extraction() {
        let path = PathBuf::from(
            "/home/user/.local/share/mise/installs/go/1.22.0/bin/go",
        );
        let version = mise_version(&path);
        assert_eq!(version.as_deref(), Some("1.22.0"));
    }

    #[test]
    fn test_nix_version_extraction() {
        let path =
            PathBuf::from("/nix/store/abc123-rust-1.78.0/bin/rustc");
        let version = nix_version(&path);
        assert_eq!(version.as_deref(), Some("1.78.0"));
    }

    #[test]
    fn test_nix_version_none_when_no_version() {
        let path =
            PathBuf::from("/nix/store/abc123-coreutils/bin/ls");
        let version = nix_version(&path);
        assert_eq!(version, None);
    }

    #[test]
    fn test_detect_mise_custom_data_dir() {
        // Custom MISE_DATA_DIR — pattern still works because we look for /installs/
        let path = PathBuf::from(
            "/opt/custom/mise/installs/python/3.12.0/bin/python3",
        );
        let source = detect_source(&path);
        match source {
            BinarySourceResolved::Mise { tool } => {
                assert_eq!(tool, "python");
            }
            _ => panic!("expected Mise source for custom data dir"),
        }
    }
}
