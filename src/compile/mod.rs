pub mod embed;
mod lower;
pub mod nickel;
pub mod plugin;

use crate::ir::types::{BesogneIR, ResolvedNativeInput, SealedSnapshot};
use crate::manifest;
use crate::probe::binary;
use std::path::{Path, PathBuf};

/// Compile a manifest into a self-contained binary.
/// Uses a content-addressed cache in $XDG_CACHE_HOME/besogne/compiled/ to avoid rebuilds.
pub fn compile(manifest_path: &Path, output_path: &Path) -> Result<(), String> {
    // 1. Parse manifest
    let mut manifest = manifest::load_manifest(manifest_path)?;

    // 1b. Expand plugins → native inputs
    if manifest.inputs.values().any(|i| matches!(i, manifest::Input::Plugin(_))) {
        let expanded = plugin::expand_plugins(&manifest, manifest_path)?;
        manifest.inputs = expanded;
    }

    // 2. Lower manifest to IR (resolve types, compute hashes)
    let mut ir = lower::lower_manifest(&manifest, manifest_path)?;

    // 2b. Build-time binary resolution — shift-left validation
    resolve_build_binaries(&mut ir)?;

    // 3. Check compile cache — key = H(compiler_binary + ir_json)
    //    Compiler change → new hash → cache miss (even if manifest unchanged)
    let ir_json = serde_json::to_vec(&ir)
        .map_err(|e| format!("cannot serialize IR for cache key: {e}"))?;
    let cache_hash = compile_cache_key(&ir_json);
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
    let mut manifest = manifest::load_manifest(manifest_path)?;
    if manifest.inputs.values().any(|i| matches!(i, manifest::Input::Plugin(_))) {
        manifest.inputs = plugin::expand_plugins(&manifest, manifest_path)?;
    }
    let mut ir = lower::lower_manifest(&manifest, manifest_path)?;
    resolve_build_binaries_quiet(&mut ir)?;

    let ir_json = serde_json::to_vec(&ir)
        .map_err(|e| format!("cannot serialize IR for cache key: {e}"))?;
    let cache_hash = compile_cache_key(&ir_json);
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

/// Parse manifest and lower to IR — no binary resolution, no compilation.
/// Used for `--help` display where we only need metadata and flags.
pub fn check_to_ir(manifest_path: &Path) -> Result<crate::ir::BesogneIR, String> {
    let mut manifest = manifest::load_manifest(manifest_path)?;
    if manifest.inputs.values().any(|i| matches!(i, manifest::Input::Plugin(_))) {
        manifest.inputs = plugin::expand_plugins(&manifest, manifest_path)?;
    }
    lower::lower_manifest(&manifest, manifest_path)
}

/// Validate a manifest without compiling
pub fn check(manifest_path: &Path) -> Result<(), String> {
    let mut manifest = manifest::load_manifest(manifest_path)?;
    if manifest.inputs.values().any(|i| matches!(i, manifest::Input::Plugin(_))) {
        manifest.inputs = plugin::expand_plugins(&manifest, manifest_path)?;
    }
    let mut ir = lower::lower_manifest(&manifest, manifest_path)?;
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

    // First pass: resolve all binaries WITHOUT parents (normal PATH resolution)
    for input in ir.inputs.iter_mut() {
        if let ResolvedNativeInput::Binary {
            name,
            path,
            version_constraint,
            parents,
            source,
            resolved_path,
            resolved_version,
            binary_hash,
        } = &mut input.input
        {
            if !parents.is_empty() {
                continue; // handled in second pass
            }

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
                    let parents_info = if !parents.is_empty() {
                        format!(" (parents: [{}])", parents.join(", "))
                    } else {
                        String::new()
                    };
                    errors.push(format!(
                        "\x1b[1;31merror\x1b[0m: binary \x1b[1m'{name}'\x1b[0m not found{parents_info}\n\
                         \x1b[1;34m  -->\x1b[0m manifest [inputs.{name}]\n\
                         \x1b[1;34m   |\x1b[0m\n\
                         \x1b[1;34m   |\x1b[0m  [inputs.{name}]\n\
                         \x1b[1;34m   |\x1b[0m  type = \"binary\"\n\
                         \x1b[1;34m   |\x1b[0m\n\
                         \x1b[1;34m   =\x1b[0m {e}\n\
                         \x1b[1;34m   =\x1b[0m \x1b[33mhint\x1b[0m: add it to PATH, set an explicit \"path\" field, or remove this input"
                    ));
                }
            }
        }
    }

    if !errors.is_empty() {
        return Err(errors.join("\n\n"));
    }

    // Collect resolved hashes by binary name (for parent lookups)
    let resolved_hashes: std::collections::HashMap<String, String> = ir.inputs.iter()
        .filter_map(|i| {
            if let ResolvedNativeInput::Binary { name, binary_hash: Some(h), parents, .. } = &i.input {
                if parents.is_empty() {
                    return Some((name.clone(), h.clone()));
                }
            }
            None
        })
        .collect();

    // Second pass: resolve binaries WITH parents (derive hash from parent hashes)
    for input in ir.inputs.iter_mut() {
        if let ResolvedNativeInput::Binary {
            name,
            parents,
            binary_hash,
            ..
        } = &mut input.input
        {
            if parents.is_empty() {
                continue; // already resolved
            }

            // Derive hash from all parent hashes
            let mut hasher = blake3::Hasher::new();
            hasher.update(b"child:");
            hasher.update(name.as_bytes());
            for parent_name in parents.iter() {
                match resolved_hashes.get(parent_name) {
                    Some(parent_hash) => {
                        hasher.update(b":");
                        hasher.update(parent_hash.as_bytes());
                    }
                    None => {
                        errors.push(format!(
                            "\x1b[1;31merror\x1b[0m: binary \x1b[1m'{name}'\x1b[0m parent not found\n\
                             \x1b[1;34m  -->\x1b[0m manifest [inputs.{name}]\n\
                             \x1b[1;34m   |\x1b[0m\n\
                             \x1b[1;34m   |\x1b[0m  parents = [\"{parent_name}\"]\n\
                             \x1b[1;34m   |\x1b[0m\n\
                             \x1b[1;34m   =\x1b[0m parent '{parent_name}' not found or not resolved\n\
                             \x1b[1;34m   =\x1b[0m \x1b[33mhint\x1b[0m: '{parent_name}' must be declared as a binary input"
                        ));
                    }
                }
            }

            if errors.is_empty() {
                let derived_hash = hasher.finalize().to_hex().to_string();
                *binary_hash = Some(derived_hash.clone());

                input.sealed = Some(SealedSnapshot {
                    hash: derived_hash,
                    size: None,
                    verified_at: chrono::Utc::now().to_rfc3339(),
                });

                if !quiet {
                    let parent_list = parents.join(", ");
                    eprintln!("besogne: resolved binary '{name}' → derived from [{parent_list}]");
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

/// Compile cache key = H(compiler_binary_hash + ir_json).
/// Ensures that ANY code change invalidates all compiled binaries,
/// even if the manifest content is identical.
fn compile_cache_key(ir_json: &[u8]) -> String {
    let compiler_hash = crate::runtime::cache::compiler_self_hash();
    let mut hasher = blake3::Hasher::new();
    hasher.update(compiler_hash.as_bytes());
    hasher.update(b":");
    hasher.update(ir_json);
    hasher.finalize().to_hex()[..32].to_string()
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
