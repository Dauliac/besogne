use super::types::Manifest;
use std::path::Path;

pub fn load_manifest(path: &Path) -> Result<Manifest, String> {
    let content = std::fs::read_to_string(path)
        .map_err(|e| format!("cannot read {}: {e}", path.display()))?;

    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();

    let manifest: Manifest = match ext.as_str() {
        "yaml" | "yml" => serde_yaml::from_str(&content)
            .map_err(|e| format!("invalid manifest YAML in {}: {e}", path.display()))?,
        "toml" => toml::from_str(&content)
            .map_err(|e| format!("invalid manifest TOML in {}: {e}", path.display()))?,
        _ => serde_json::from_str(&content)
            .map_err(|e| format!("invalid manifest JSON in {}: {e}", path.display()))?,
    };

    validate_manifest(&manifest)?;

    Ok(manifest)
}

/// Discover manifest files in the current directory or git root.
/// Returns found paths, or empty vec if none found.
pub fn discover_manifests() -> Vec<std::path::PathBuf> {
    let cwd = std::env::current_dir().unwrap_or_default();
    let mut found = Vec::new();

    // 1. Check current dir for well-known names
    for name in &["besogne.json", "besogne.yaml", "besogne.yml", "besogne.toml",
                   ".besogne.json", ".besogne.yaml", ".besogne.yml", ".besogne.toml"] {
        let p = cwd.join(name);
        if p.is_file() {
            found.push(p);
        }
    }

    // 2. Glob *.besogne.{json,yaml,yml,toml} in current dir
    if let Ok(entries) = std::fs::read_dir(&cwd) {
        for entry in entries.flatten() {
            let path = entry.path();
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                let lower = name.to_lowercase();
                if (lower.ends_with(".besogne.json")
                    || lower.ends_with(".besogne.yaml")
                    || lower.ends_with(".besogne.yml")
                    || lower.ends_with(".besogne.toml"))
                    && !found.contains(&path)
                {
                    found.push(path);
                }
            }
        }
    }

    // 3. If nothing in cwd, try git root
    if found.is_empty() {
        if let Some(git_root) = find_git_root(&cwd) {
            if git_root != cwd {
                for name in &["besogne.json", "besogne.yaml", "besogne.yml", "besogne.toml"] {
                    let p = git_root.join(name);
                    if p.is_file() {
                        found.push(p);
                    }
                }
                if let Some(repo_name) = git_root.file_name().and_then(|n| n.to_str()) {
                    for ext in &["json", "yaml", "yml", "toml"] {
                        let p = git_root.join(format!("{repo_name}.besogne.{ext}"));
                        if p.is_file() && !found.contains(&p) {
                            found.push(p);
                        }
                    }
                }
            }
        }
    }

    // 4. Check besogne/ directory for terminal manifests
    let besogne_dir = cwd.join("besogne");
    if besogne_dir.is_dir() {
        if let Ok(entries) = std::fs::read_dir(&besogne_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                    if matches!(ext, "toml" | "json" | "yaml" | "yml")
                        && path.is_file()
                        && !found.contains(&path)
                    {
                        found.push(path);
                    }
                }
            }
        }
    }

    found.sort();
    found.dedup();
    found
}

fn find_git_root(start: &Path) -> Option<std::path::PathBuf> {
    let mut dir = start.to_path_buf();
    loop {
        if dir.join(".git").exists() {
            return Some(dir);
        }
        if !dir.pop() {
            return None;
        }
    }
}

fn validate_manifest(manifest: &Manifest) -> Result<(), String> {
    if manifest.name.is_empty() {
        return Err("manifest 'name' is required".into());
    }

    // With named map, duplicates are impossible (map keys are unique).
    // Validate exec-phase commands have a `run` field (always required by serde).
    // No other structural validation needed — the map key IS the name.

    Ok(())
}
