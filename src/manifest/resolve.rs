use crate::error::BesogneError;
use crate::manifest;
use std::path::PathBuf;

/// Resolve inputs: use explicit paths or auto-discover.
pub fn resolve_manifests(explicit: &[PathBuf]) -> Result<Vec<PathBuf>, BesogneError> {
    if !explicit.is_empty() {
        return Ok(explicit.to_vec());
    }
    let discovered = manifest::discover_manifests();
    if discovered.is_empty() {
        return Err(BesogneError::Cli(
            "no manifest found. Provide --input or create a besogne.{json,yaml,yml,toml} file."
                .into(),
        ));
    }
    eprintln!(
        "besogne: discovered {}",
        discovered
            .iter()
            .map(|p| p.display().to_string())
            .collect::<Vec<_>>()
            .join(", ")
    );
    Ok(discovered)
}

/// Resolve a single manifest for `run`. Supports task name selection from args.
/// When multiple manifests found, checks if `args[0]` matches a task name (stem of a manifest).
/// Returns (manifest_path, remaining_args).
pub fn resolve_single_manifest<'a>(
    explicit: &Option<PathBuf>,
    args: &'a [String],
) -> Result<(PathBuf, &'a [String]), BesogneError> {
    if let Some(p) = explicit {
        return Ok((p.clone(), args));
    }
    let discovered = manifest::discover_manifests();
    match discovered.len() {
        0 => Err(BesogneError::Cli(
            "no manifest found. Provide --input or create a besogne.{json,yaml,yml,toml} file."
                .into(),
        )),
        1 => Ok((discovered[0].clone(), args)),
        _ => {
            // Try to match args[0] as a task name
            if let Some(task_name) = args.first() {
                if !task_name.starts_with('-') {
                    for path in &discovered {
                        let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
                        let name = stem.strip_suffix(".besogne").unwrap_or(stem);
                        if name == task_name {
                            return Ok((path.clone(), &args[1..]));
                        }
                    }
                }
            }

            let names: Vec<String> = discovered
                .iter()
                .filter_map(|p| {
                    let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or("");
                    Some(stem.strip_suffix(".besogne").unwrap_or(stem).to_string())
                })
                .collect();
            Err(BesogneError::Cli(format!(
                "multiple manifests found — specify which task to run:\n  besogne run <task> [-- args]\n\navailable tasks:\n  {}",
                names.join("\n  ")
            )))
        }
    }
}

/// Extract the task name from a manifest path (stem without .besogne suffix).
pub fn manifest_task_name(path: &std::path::Path) -> String {
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("besogne");
    stem.strip_suffix(".besogne").unwrap_or(stem).to_string()
}
