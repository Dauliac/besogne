pub mod embed;
mod lower;
pub mod plugin;

use crate::ir::types::{BesogneIR, ResolvedNativeInput, SealedSnapshot};
use crate::manifest;
use crate::probe::binary;
use std::path::{Path, PathBuf};

/// Compile a manifest into a self-contained binary.
/// Uses a content-addressed cache in $XDG_CACHE_HOME/besogne/compiled/ to avoid rebuilds.
pub fn compile(manifest_path: &Path, output_path: &Path) -> Result<(), String> {
    // 1. Parse manifest
    let manifest = manifest::load_manifest(manifest_path)?;

    // 2. Lower manifest to IR (resolve types, compute hashes)
    let mut ir = lower::lower_manifest(&manifest)?;

    // 2b. Build-time binary resolution — shift-left validation
    resolve_build_binaries(&mut ir)?;

    // 3. Check compile cache
    let ir_json = serde_json::to_vec(&ir)
        .map_err(|e| format!("cannot serialize IR for cache key: {e}"))?;
    let cache_hash = blake3::hash(&ir_json).to_hex()[..32].to_string();
    let cached_path = compile_cache_path(&cache_hash);

    if cached_path.exists() {
        // Cache hit — copy cached binary to output
        std::fs::copy(&cached_path, output_path)
            .map_err(|e| format!("cannot copy cached binary: {e}"))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = std::fs::Permissions::from_mode(0o755);
            std::fs::set_permissions(output_path, perms)
                .map_err(|e| format!("cannot set permissions: {e}"))?;
        }
        eprintln!("besogne: cache hit ({cache_hash})");
        return Ok(());
    }

    // 4. Cache miss — emit binary
    embed::emit(output_path, &ir)?;

    // 5. Store in cache
    if let Some(parent) = cached_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::copy(output_path, &cached_path);

    Ok(())
}

/// Compile without progress messages (for `besogne run` where the binary handles output)
pub fn compile_quiet(manifest_path: &Path, output_path: &Path) -> Result<(), String> {
    let manifest = manifest::load_manifest(manifest_path)?;
    let mut ir = lower::lower_manifest(&manifest)?;
    resolve_build_binaries_quiet(&mut ir)?;

    let ir_json = serde_json::to_vec(&ir)
        .map_err(|e| format!("cannot serialize IR for cache key: {e}"))?;
    let cache_hash = blake3::hash(&ir_json).to_hex()[..32].to_string();
    let cached_path = compile_cache_path(&cache_hash);

    if cached_path.exists() {
        std::fs::copy(&cached_path, output_path)
            .map_err(|e| format!("cannot copy cached binary: {e}"))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = std::fs::Permissions::from_mode(0o755);
            std::fs::set_permissions(output_path, perms)
                .map_err(|e| format!("cannot set permissions: {e}"))?;
        }
        return Ok(());
    }

    embed::emit(output_path, &ir)?;

    if let Some(parent) = cached_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::copy(output_path, &cached_path);

    Ok(())
}

/// Validate a manifest without compiling
pub fn check(manifest_path: &Path) -> Result<(), String> {
    let manifest = manifest::load_manifest(manifest_path)?;
    let mut ir = lower::lower_manifest(&manifest)?;
    resolve_build_binaries(&mut ir)?;
    Ok(())
}

/// Quiet variant — same resolution, no eprintln output
fn resolve_build_binaries_quiet(ir: &mut BesogneIR) -> Result<(), String> {
    resolve_build_binaries_inner(ir, true)
}

/// Build-time binary resolution: resolve all binary inputs, detect source, extract version, hash.
fn resolve_build_binaries(ir: &mut BesogneIR) -> Result<(), String> {
    resolve_build_binaries_inner(ir, false)
}

fn resolve_build_binaries_inner(ir: &mut BesogneIR, quiet: bool) -> Result<(), String> {
    let mut errors = Vec::new();

    for input in &mut ir.inputs {
        if let ResolvedNativeInput::Binary {
            name,
            path,
            version_constraint,
            source,
            resolved_path,
            resolved_version,
            binary_hash,
        } = &mut input.input
        {
            let has_version_field = version_constraint.is_some();

            match binary::resolve_binary(name, path.as_deref(), has_version_field) {
                Ok(resolved) => {
                    *source = Some(resolved.source);
                    *resolved_path =
                        Some(resolved.canonical_path.to_string_lossy().to_string());
                    *resolved_version = resolved.version;
                    *binary_hash = Some(resolved.hash.clone());

                    // Seal the binary snapshot
                    let size = std::fs::metadata(&resolved.canonical_path)
                        .ok()
                        .map(|m| m.len());
                    input.sealed = Some(SealedSnapshot {
                        hash: resolved.hash,
                        size,
                        verified_at: chrono::Utc::now().to_rfc3339(),
                    });

                    if !quiet {
                        eprintln!(
                            "besogne: resolved binary '{}' → {} [{}]",
                            name,
                            resolved.canonical_path.display(),
                            match &*source {
                                Some(crate::ir::types::BinarySourceResolved::Nix { pname, .. }) =>
                                    format!("nix:{}", pname.as_deref().unwrap_or("?")),
                                Some(crate::ir::types::BinarySourceResolved::Mise { tool }) =>
                                    format!("mise:{tool}"),
                                Some(crate::ir::types::BinarySourceResolved::System) =>
                                    "system".to_string(),
                                None => "unknown".to_string(),
                            },
                        );

                        if let Some(ver) = resolved_version.as_ref() {
                            eprintln!("  version: {ver}");
                        }
                    }
                }
                Err(e) => {
                    errors.push(format!("binary '{name}': {e}"));
                }
            }
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "build-time binary resolution failed:\n  {}",
            errors.join("\n  ")
        ))
    }
}

fn compile_cache_path(hash: &str) -> PathBuf {
    let cache_dir = std::env::var("XDG_CACHE_HOME").unwrap_or_else(|_| {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
        format!("{home}/.cache")
    });
    Path::new(&cache_dir)
        .join("besogne")
        .join("compiled")
        .join(hash)
}
