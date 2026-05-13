//! `besogne adopt` — migrate project task runners into besogne manifests.
//!
//! Parses package.json scripts (and future: composer.json, Makefile, mise.toml, etc.),
//! extracts binaries via shell command analysis, detects impure patterns,
//! generates a besogne.toml, backs up the original, and rewrites scripts to `besogne run`.

mod impurity;
mod npm;
mod toml_gen;

use crate::error::BesogneError;
use std::path::{Path, PathBuf};

/// Source format to adopt from
#[derive(Debug, Clone)]
pub enum AdoptSource {
    PackageJson,
    // Future: ComposerJson, Makefile, MiseToml, Taskfile, Justfile
}

/// A parsed script from the source file
#[derive(Debug, Clone)]
pub struct ParsedScript {
    pub name: String,
    pub body: String,
    /// Binaries discovered in the script body
    pub binaries: Vec<String>,
    /// Environment variables referenced ($VAR)
    pub env_vars: Vec<String>,
    /// npm lifecycle ordering (pre/post)
    pub parents: Vec<String>,
    /// Detected as impure by heuristics
    pub side_effects: bool,
}

/// Result of adopting a source file
#[derive(Debug)]
pub struct AdoptResult {
    pub manifest_path: PathBuf,
    pub backup_path: PathBuf,
    pub scripts: Vec<ParsedScript>,
}

/// Run the full adopt pipeline
pub fn adopt(
    source_path: &Path,
    source_type: &AdoptSource,
    output_path: &Path,
    dry_run: bool,
) -> Result<AdoptResult, BesogneError> {
    match source_type {
        AdoptSource::PackageJson => adopt_package_json(source_path, output_path, dry_run),
    }
}

fn adopt_package_json(
    source_path: &Path,
    output_path: &Path,
    dry_run: bool,
) -> Result<AdoptResult, BesogneError> {
    // 1. Parse package.json scripts
    let (pkg_value, mut scripts) = npm::parse_package_json(source_path)?;

    // 2. Analyze each script for binaries and impurity
    for script in &mut scripts {
        let extracted = extract_commands(&script.body);
        script.binaries = extracted.binaries;
        script.env_vars = extracted.env_vars;
        script.side_effects = impurity::detect_impurity(&script.body, &script.name, &extracted.commands);
    }

    // 3. Resolve npm lifecycle ordering (pre/post scripts)
    npm::resolve_lifecycle_ordering(&mut scripts);

    // 4. Generate besogne.toml
    let toml_content = toml_gen::generate_toml(&scripts, source_path)?;

    if dry_run {
        eprintln!("besogne adopt: dry run — would generate:\n");
        eprintln!("{toml_content}");
        return Ok(AdoptResult {
            manifest_path: output_path.to_path_buf(),
            backup_path: backup_path(source_path),
            scripts,
        });
    }

    // 5. Write besogne.toml
    std::fs::write(output_path, &toml_content)
        .map_err(|e| BesogneError::Adopt(format!("cannot write {}: {e}", output_path.display())))?;
    eprintln!("besogne adopt: wrote {}", output_path.display());

    // 6. Backup original
    let backup = backup_path(source_path);
    std::fs::copy(source_path, &backup)
        .map_err(|e| BesogneError::Adopt(format!("cannot backup {}: {e}", source_path.display())))?;
    eprintln!("besogne adopt: backed up → {}", backup.display());

    // 7. Rewrite package.json scripts to `besogne run <name>`
    let rewritten = npm::rewrite_package_json(&pkg_value, &scripts)?;
    std::fs::write(source_path, &rewritten)
        .map_err(|e| BesogneError::Adopt(format!("cannot rewrite {}: {e}", source_path.display())))?;
    eprintln!("besogne adopt: rewrote {}", source_path.display());

    Ok(AdoptResult {
        manifest_path: output_path.to_path_buf(),
        backup_path: backup,
        scripts,
    })
}

/// Generate backup path: package.json → package.besogne.old.json
fn backup_path(source_path: &Path) -> PathBuf {
    let stem = source_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("source");
    let ext = source_path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("");
    let backup_name = if ext.is_empty() {
        format!("{stem}.besogne.old")
    } else {
        format!("{stem}.besogne.old.{ext}")
    };
    source_path
        .parent()
        .unwrap_or(Path::new("."))
        .join(backup_name)
}

/// Extracted command info from a script body
struct ExtractedCommands {
    binaries: Vec<String>,
    env_vars: Vec<String>,
    commands: Vec<String>,
}

/// Extract command names and env vars from a shell script body.
/// Simple lexical analysis (tree-sitter-bash is not yet available).
fn extract_commands(body: &str) -> ExtractedCommands {
    let mut binaries = Vec::new();
    let mut env_vars = Vec::new();
    let mut commands = Vec::new();

    // Split on shell operators
    let segments: Vec<&str> = body
        .split(|c: char| matches!(c, '&' | '|' | ';' | '\n'))
        .collect();

    for segment in segments {
        let segment = segment.trim();
        if segment.is_empty() {
            continue;
        }

        // Skip inline env assignments at start (KEY=val cmd ...)
        let mut words: Vec<&str> = segment.split_whitespace().collect();

        // Remove leading env assignments
        while !words.is_empty() && words[0].contains('=') && !words[0].starts_with('-') {
            words.remove(0);
        }

        if let Some(cmd) = words.first() {
            let cmd = *cmd;
            // Skip shell builtins
            if !matches!(
                cmd,
                "if" | "then" | "else" | "fi" | "for" | "do" | "done"
                    | "while" | "until" | "case" | "esac" | "true" | "false"
                    | "exit" | "return" | "echo" | "printf" | "cd" | "["
                    | "test" | "set" | "export" | "source" | "."
            ) {
                // Resolve npx/bunx → the actual command
                if matches!(cmd, "npx" | "bunx" | "pnpx") {
                    if let Some(actual) = words.get(1) {
                        if !actual.starts_with('-') {
                            commands.push(actual.to_string());
                            if !binaries.contains(&actual.to_string()) {
                                binaries.push(actual.to_string());
                            }
                        }
                    }
                } else {
                    commands.push(cmd.to_string());
                    if !binaries.contains(&cmd.to_string()) {
                        binaries.push(cmd.to_string());
                    }
                }
            }
        }

        // Extract $VAR references
        let mut chars = segment.chars().peekable();
        while let Some(c) = chars.next() {
            if c == '$' {
                // Skip ${ ... }
                if chars.peek() == Some(&'{') {
                    chars.next();
                    let var: String = chars
                        .by_ref()
                        .take_while(|&c| c != '}')
                        .collect();
                    if !var.is_empty() && !env_vars.contains(&var) {
                        env_vars.push(var);
                    }
                } else {
                    let var: String = chars
                        .by_ref()
                        .take_while(|c| c.is_alphanumeric() || *c == '_')
                        .collect();
                    if !var.is_empty() && !env_vars.contains(&var) {
                        env_vars.push(var);
                    }
                }
            }
        }
    }

    ExtractedCommands {
        binaries,
        env_vars,
        commands,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_commands_simple() {
        let result = extract_commands("tsc && webpack --mode production");
        assert!(result.binaries.contains(&"tsc".to_string()));
        assert!(result.binaries.contains(&"webpack".to_string()));
    }

    #[test]
    fn test_extract_commands_env_prefix() {
        let result = extract_commands("NODE_ENV=production webpack");
        assert!(result.binaries.contains(&"webpack".to_string()));
        assert!(!result.binaries.contains(&"NODE_ENV=production".to_string()));
    }

    #[test]
    fn test_extract_commands_npx() {
        let result = extract_commands("npx jest --coverage");
        assert!(result.binaries.contains(&"jest".to_string()));
    }

    #[test]
    fn test_extract_env_vars() {
        let result = extract_commands("echo $HOME ${NODE_ENV}");
        assert!(result.env_vars.contains(&"HOME".to_string()));
        assert!(result.env_vars.contains(&"NODE_ENV".to_string()));
    }

    #[test]
    fn test_backup_path() {
        let p = backup_path(Path::new("/app/package.json"));
        assert_eq!(p, PathBuf::from("/app/package.besogne.old.json"));
    }

    #[test]
    fn test_backup_path_no_ext() {
        let p = backup_path(Path::new("/app/Makefile"));
        assert_eq!(p, PathBuf::from("/app/Makefile.besogne.old"));
    }
}
