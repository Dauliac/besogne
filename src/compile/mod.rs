pub mod embed;
mod lower;
pub mod component;

use crate::ir::types::{BesogneIR, ResolvedNativeNode, SealedSnapshot};
use crate::manifest::{self, Phase};
use crate::output::style::{self, DiagBuilder};
use crate::probe::binary;
use std::path::{Path, PathBuf};
use std::time::Instant;

/// Compile a manifest into a self-contained binary in the global store.
/// Returns the store path of the binary.
/// Uses a content-addressed cache in $XDG_CACHE_HOME/besogne/store/ — same IR = same binary.
pub fn compile(manifest_path: &Path, output_path: &Path, force: bool) -> Result<PathBuf, String> {
    let build_start = Instant::now();

    use crate::output::style::l3;
    use crate::output::style::phase::Phase as StylePhase;

    // 0. Quick check: if output binary exists and manifest hasn't changed, skip entirely.
    if !force {
        if let Some(store_path) = check_build_lock(manifest_path, output_path) {
            return Ok(store_path);
        }
    }

    // 1. Parse manifest
    let step = Instant::now();
    let mut manifest = manifest::load_manifest(manifest_path)?;
    let parse_ms = step.elapsed().as_millis();

    // Show phase banner after parsing so we know the node count
    let total_nodes = manifest.nodes.len();
    eprintln!("\n{}", l3::sections::phase_banner::render(
        StylePhase::Build, total_nodes, None));
    eprintln!("{}", l3::items::progress_step::render(
        &format!("parsed {} ({}ms)", manifest_path.display(), parse_ms)));

    // 1b. Expand components → native inputs
    let component_count = manifest.nodes.values()
        .filter(|i| matches!(i, manifest::Node::Component(_)))
        .count();
    if component_count > 0 {
        let step = Instant::now();
        let expanded = component::expand_components(&manifest, manifest_path)?;
        let expand_ms = step.elapsed().as_millis();
        let produced = expanded.len();
        manifest.nodes = expanded;
        eprintln!("{}", l3::items::progress_step::render(
            &format!("expanded {} component → {} nodes ({}ms)", component_count, produced, expand_ms)));
    }

    // 2. Lower manifest to IR (resolve types, compute hashes)
    let step = Instant::now();
    let mut ir = lower::lower_manifest(&manifest, manifest_path)?;
    let lower_ms = step.elapsed().as_millis();
    let node_summary = build_node_summary(&ir);
    eprintln!("{}", l3::items::progress_step::render(
        &format!("lowered {node_summary} ({}ms)", lower_ms)));

    // 2b. Build-time binary resolution — shift-left validation
    let step = Instant::now();
    let pin_result = resolve_build_binaries_inner(&mut ir, true);
    let pin_ms = step.elapsed().as_millis();
    let pin_summary = build_pin_summary(&ir);
    eprintln!("{}", l3::items::progress_step::render(
        &format!("pinned {pin_summary} ({}ms)", pin_ms)));
    pin_result?;

    // 3. Check content-addressed store
    // IR is fully deterministic (no timestamps) — same manifest + same binaries = same hash.
    let ir_json = serde_json::to_vec(&ir)
        .map_err(|e| format!("cannot serialize IR for cache key: {e}"))?;
    let cache_hash = compile_cache_key(&ir_json);
    let store_binary = store_binary_path(&cache_hash);

    // Ensure output directory exists
    if let Some(parent) = output_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    if !force && store_binary.exists() {
        // Store hit — copy or symlink to output
        std::fs::copy(&store_binary, output_path)
            .map_err(|e| format!("cannot copy from store: {e}"))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = std::fs::Permissions::from_mode(0o755);
            std::fs::set_permissions(output_path, perms)
                .map_err(|e| format!("cannot set permissions: {e}"))?;
        }
        let total_ms = build_start.elapsed().as_millis();
        eprintln!("{}", l3::items::progress_step::render(
            &format!("store hit {} ({}ms total)", &cache_hash[..16], total_ms)));
        eprintln!("  {}", l3::sections::footer_line::render(0, total_ms as u64));
        write_build_lock(manifest_path, output_path, &cache_hash);
        return Ok(store_binary);
    }

    // 4. Store miss — emit binary directly to store
    let step = Instant::now();
    if let Some(parent) = store_binary.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    embed::emit(&store_binary, &ir)?;
    let emit_ms = step.elapsed().as_millis();

    // 5. Write metadata sidecar (provenance + Nix store compatibility)
    write_store_metadata(&store_binary, &ir, &cache_hash, manifest_path);

    // 6. Copy to output path
    std::fs::copy(&store_binary, output_path)
        .map_err(|e| format!("cannot copy to output: {e}"))?;

    let binary_size = std::fs::metadata(&store_binary).ok().map(|m| m.len()).unwrap_or(0);
    let total_ms = build_start.elapsed().as_millis();
    eprintln!("{}", l3::items::progress_step::render(
        &format!("emitted {} ({}, {}ms, {}ms total)", &cache_hash[..16], format_size(binary_size), emit_ms, total_ms)));
    eprintln!("  {}", l3::sections::footer_line::render(0, total_ms as u64));
    write_build_lock(manifest_path, output_path, &cache_hash);

    Ok(store_binary)
}

/// Compile with minimal progress. Returns the store path of the binary.
/// Does NOT copy to output_path — caller should create a symlink or copy.
pub fn compile_quiet(manifest_path: &Path) -> Result<PathBuf, String> {
    let build_start = Instant::now();
    let mut manifest = manifest::load_manifest(manifest_path)?;
    if manifest.nodes.values().any(|i| matches!(i, manifest::Node::Component(_))) {
        manifest.nodes = component::expand_components(&manifest, manifest_path)?;
    }
    let mut ir = lower::lower_manifest(&manifest, manifest_path)?;
    resolve_build_binaries_quiet(&mut ir)?;

    let ir_json = serde_json::to_vec(&ir)
        .map_err(|e| format!("cannot serialize IR for cache key: {e}"))?;
    let cache_hash = compile_cache_key(&ir_json);
    let store_bin = store_binary_path(&cache_hash);

    if store_bin.exists() {
        return Ok(store_bin);
    }

    if let Some(parent) = store_bin.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    embed::emit(&store_bin, &ir)?;
    write_store_metadata(&store_bin, &ir, &cache_hash, manifest_path);

    let total_ms = build_start.elapsed().as_millis();
    let node_summary = build_node_summary(&ir);
    {
        use crate::output::style::l3;
        use crate::output::style::phase::Phase as StylePhase;
        let total_nodes = ir.nodes.len();
        eprintln!("{}", l3::sections::phase_banner::render(
            StylePhase::Build, total_nodes, None));
        eprintln!("{}", l3::items::progress_step::render(
            &format!("built {node_summary} ({}ms)", total_ms)));
        eprintln!("  {}", l3::sections::footer_line::render(0, total_ms as u64));
    }

    Ok(store_bin)
}

/// Parse manifest and lower to IR — no binary resolution, no compilation.
/// Used for `--help` display where we only need metadata and flags.
pub fn check_to_ir(manifest_path: &Path) -> Result<crate::ir::BesogneIR, String> {
    let mut manifest = manifest::load_manifest(manifest_path)?;
    if manifest.nodes.values().any(|i| matches!(i, manifest::Node::Component(_))) {
        manifest.nodes = component::expand_components(&manifest, manifest_path)?;
    }
    lower::lower_manifest(&manifest, manifest_path)
}

/// Validate a manifest without compiling
pub fn check(manifest_path: &Path) -> Result<(), String> {
    let mut manifest = manifest::load_manifest(manifest_path)?;
    if manifest.nodes.values().any(|i| matches!(i, manifest::Node::Component(_))) {
        manifest.nodes = component::expand_components(&manifest, manifest_path)?;
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
    use std::sync::Mutex;

    // Collect binary resolution tasks (index, name, path, has_version)
    let tasks: Vec<(usize, String, Option<String>, bool)> = ir.nodes.iter().enumerate()
        .filter_map(|(idx, input)| {
            if let ResolvedNativeNode::Binary { name, path, version_constraint, parents, .. } = &input.node {
                if parents.is_empty() {
                    return Some((idx, name.clone(), path.clone(), version_constraint.is_some()));
                }
            }
            None
        })
        .collect();

    // Resolve all binaries in parallel with shared hash cache
    // (many binaries like coreutils share the same canonical path)
    let hash_cache: Mutex<std::collections::HashMap<std::path::PathBuf, String>> =
        Mutex::new(std::collections::HashMap::new());
    let results: Mutex<Vec<(usize, Result<binary::ResolvedBinary, String>)>> =
        Mutex::new(Vec::with_capacity(tasks.len()));

    crossbeam::scope(|s| {
        for (idx, name, path, has_version) in &tasks {
            let results = &results;
            let hash_cache = &hash_cache;
            s.spawn(move |_| {
                let resolved = binary::resolve_binary_with_cache(
                    name, path.as_deref(), *has_version, Some(hash_cache));
                results.lock().unwrap().push((*idx, resolved));
            });
        }
    }).unwrap();

    // Apply results back to IR
    let mut errors = Vec::new();
    for (idx, result) in results.into_inner().unwrap() {
        let input = &mut ir.nodes[idx];
        match result {
            Ok(resolved) => {
                if let ResolvedNativeNode::Binary {
                    name, source, resolved_path, resolved_version, binary_hash, ..
                } = &mut input.node {
                    *source = Some(resolved.source);
                    *resolved_path = Some(resolved.canonical_path.to_string_lossy().to_string());
                    *resolved_version = resolved.version;
                    *binary_hash = Some(resolved.hash.clone());

                    let size = std::fs::metadata(&resolved.canonical_path)
                        .ok()
                        .map(|m| m.len());
                    input.sealed = Some(SealedSnapshot {
                        hash: resolved.hash,
                        size,
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
            }
            Err(e) => {
                if let ResolvedNativeNode::Binary { name, parents, .. } = &input.node {
                    let parents_info = if !parents.is_empty() {
                        format!(" (parents: [{}])", parents.join(", "))
                    } else {
                        String::new()
                    };
                    let header = style::error_diag(&format!("binary {} not found{parents_info}", style::bold(name)));
                    let body = DiagBuilder::new()
                        .location(&format!("manifest [nodes.{name}]"))
                        .blank()
                        .code(&format!("[nodes.{name}]"))
                        .code("type = \"binary\"")
                        .blank()
                        .note(&e.to_string())
                        .hint("add it to PATH, set an explicit \"path\" field, or remove this node")
                        .build();
                    errors.push(format!("{header}\n{body}"));
                }
            }
        }
    }

    if !errors.is_empty() {
        return Err(errors.join("\n\n"));
    }

    // Collect resolved hashes by binary name AND qualified key (for parent lookups).
    // Component-expanded nodes have parents like "node/toolchain.node" (qualified key)
    // while the binary name is just "node". Index by both so either reference works.
    let mut resolved_hashes: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    for i in ir.nodes.iter() {
        if let ResolvedNativeNode::Binary { name, binary_hash: Some(h), parents, .. } = &i.node {
            if parents.is_empty() {
                resolved_hashes.insert(name.clone(), h.clone());
                // Also index by the qualified key from ContentId (e.g., "node/toolchain.node")
                let qualified = i.id.0.split(':').nth(1).unwrap_or("");
                if qualified != name {
                    resolved_hashes.insert(qualified.to_string(), h.clone());
                }
            }
        }
    }

    // Second pass: resolve binaries WITH parents (derive hash from parent hashes)
    for input in ir.nodes.iter_mut() {
        if let ResolvedNativeNode::Binary {
            name,
            parents,
            binary_hash,
            ..
        } = &mut input.node
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
                        let header = style::error_diag(&format!("binary {} parent not found", style::bold(name)));
                        let body = DiagBuilder::new()
                            .location(&format!("manifest [nodes.{name}]"))
                            .blank()
                            .code(&format!("parents = [\"{parent_name}\"]"))
                            .blank()
                            .note(&format!("parent '{parent_name}' not found or not resolved"))
                            .hint(&format!("'{parent_name}' must be declared as a binary node"))
                            .build();
                        errors.push(format!("{header}\n{body}"));
                    }
                }
            }

            if errors.is_empty() {
                let derived_hash = hasher.finalize().to_hex().to_string();
                *binary_hash = Some(derived_hash.clone());

                input.sealed = Some(SealedSnapshot {
                    hash: derived_hash,
                    size: None,
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

/// Write metadata sidecar alongside the binary in the store.
/// Contains provenance info and Nix store references for future `besogne push`.
fn write_store_metadata(
    store_binary: &Path,
    ir: &BesogneIR,
    cache_hash: &str,
    manifest_path: &Path,
) {
    use crate::ir::types::{ResolvedNativeNode, BinarySourceResolved};

    let store_dir = store_binary.parent().unwrap_or(Path::new("."));
    let metadata_path = store_dir.join("metadata.json");

    // Extract Nix store references from sealed binaries
    let mut nix_references: Vec<String> = Vec::new();
    let mut sealed_binaries = serde_json::Map::new();
    let mut components: Vec<String> = Vec::new();

    for node in &ir.nodes {
        // Collect Nix references from binary nodes
        if let ResolvedNativeNode::Binary { name, source, resolved_path, binary_hash, .. } = &node.node {
            let mut entry = serde_json::Map::new();
            if let Some(hash) = binary_hash {
                entry.insert("hash".into(), serde_json::Value::String(hash.clone()));
            }
            if let Some(path) = resolved_path {
                entry.insert("resolved_path".into(), serde_json::Value::String(path.clone()));
            }
            if let Some(BinarySourceResolved::Nix { store_path, pname }) = source {
                entry.insert("nix_store_path".into(), serde_json::Value::String(store_path.clone()));
                if let Some(pname) = pname {
                    entry.insert("nix_pname".into(), serde_json::Value::String(pname.clone()));
                }
                // Extract the derivation path (everything up to /bin/)
                let drv_path = store_path.split("/bin/").next().unwrap_or(store_path);
                if !nix_references.contains(&drv_path.to_string()) {
                    nix_references.push(drv_path.to_string());
                }
            }
            sealed_binaries.insert(name.clone(), serde_json::Value::Object(entry));
        }

        // Collect component origins
        if let Some(comp) = &node.from_component {
            if !components.contains(comp) {
                components.push(comp.clone());
            }
        }
    }

    nix_references.sort();
    components.sort();

    let system = std::env::consts::ARCH.to_string() + "-" + std::env::consts::OS;

    let metadata = serde_json::json!({
        "name": ir.metadata.name,
        "description": ir.metadata.description,
        "blake3": cache_hash,
        "built_at": chrono::Utc::now().to_rfc3339(),
        "manifest_path": manifest_path.display().to_string(),
        "compiler_hash": crate::runtime::cache::compiler_self_hash(),
        "components": components,
        "sealed_binaries": sealed_binaries,
        "nix": {
            "system": system,
            "references": nix_references,
        }
    });

    // Best-effort write — metadata is non-critical
    if let Ok(json) = serde_json::to_string_pretty(&metadata) {
        let _ = std::fs::write(&metadata_path, json);
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

/// Content-addressed store path: ~/.cache/besogne/store/{hash}/binary
fn store_binary_path(hash: &str) -> PathBuf {
    crate::runtime::cache::cache_base_dir()
        .join("store")
        .join(hash)
        .join("binary")
}

/// Summarize IR nodes by phase: "3 build, 2 seal, 4 exec"
fn build_node_summary(ir: &BesogneIR) -> String {
    let build = ir.nodes.iter().filter(|n| n.phase == Phase::Build).count();
    let seal = ir.nodes.iter().filter(|n| n.phase == Phase::Seal).count();
    let exec = ir.nodes.iter().filter(|n| n.phase == Phase::Exec).count();
    let mut parts = Vec::new();
    if build > 0 { parts.push(format!("{build} build")); }
    if seal > 0 { parts.push(format!("{seal} seal")); }
    if exec > 0 { parts.push(format!("{exec} exec")); }
    parts.join(", ")
}

/// Summarize pinned binaries by source: "32 nix, 1 system, 2 mise"
fn build_pin_summary(ir: &BesogneIR) -> String {
    let mut nix = 0usize;
    let mut mise = 0usize;
    let mut system = 0usize;
    let mut other = 0usize;
    for node in &ir.nodes {
        if let ResolvedNativeNode::Binary { source, binary_hash, .. } = &node.node {
            if binary_hash.is_none() { continue; }
            match source {
                Some(crate::ir::types::BinarySourceResolved::Nix { .. }) => nix += 1,
                Some(crate::ir::types::BinarySourceResolved::Mise { .. }) => mise += 1,
                Some(crate::ir::types::BinarySourceResolved::System) => system += 1,
                None => other += 1,
            }
        }
    }
    let total = nix + mise + system + other;
    let mut parts = vec![format!("{total} binaries")];
    let mut sources = Vec::new();
    if nix > 0 { sources.push(format!("{nix} nix")); }
    if system > 0 { sources.push(format!("{system} system")); }
    if mise > 0 { sources.push(format!("{mise} mise")); }
    if other > 0 { sources.push(format!("{other} other")); }
    if !sources.is_empty() {
        parts.push(format!("({})", sources.join(", ")));
    }
    parts.join(" ")
}

fn format_size(bytes: u64) -> String {
    if bytes >= 1_048_576 {
        format!("{:.1}MB", bytes as f64 / 1_048_576.0)
    } else if bytes >= 1024 {
        format!("{:.1}KB", bytes as f64 / 1024.0)
    } else {
        format!("{bytes}B")
    }
}

/// Build lock: maps manifest content hash → store hash.
/// Stored next to the output binary as `<output>.lock`.
/// Allows skipping the entire build pipeline when nothing changed.
fn build_lock_path(output_path: &Path) -> PathBuf {
    let mut p = output_path.to_path_buf();
    let name = p.file_name().unwrap_or_default().to_string_lossy().to_string();
    p.set_file_name(format!("{name}.lock"));
    p
}

/// Hash the manifest file + all component files it references.
/// This is a cheap check (just file content hashing, no parsing).
fn manifest_content_hash(manifest_path: &Path) -> Option<String> {
    let content = std::fs::read(manifest_path).ok()?;
    Some(blake3::hash(&content).to_hex().to_string())
}

/// Check if the build lock is valid: manifest unchanged AND store binary exists.
fn check_build_lock(manifest_path: &Path, output_path: &Path) -> Option<PathBuf> {
    let lock_path = build_lock_path(output_path);
    let lock_content = std::fs::read_to_string(&lock_path).ok()?;
    let lock: serde_json::Value = serde_json::from_str(&lock_content).ok()?;

    let cached_manifest_hash = lock.get("manifest_hash")?.as_str()?;
    let cached_store_hash = lock.get("store_hash")?.as_str()?;

    // Check manifest content unchanged
    let current_hash = manifest_content_hash(manifest_path)?;
    if current_hash != cached_manifest_hash {
        return None;
    }

    // Check store binary still exists
    let store_bin = store_binary_path(cached_store_hash);
    if !store_bin.exists() {
        return None;
    }

    // Check output binary still exists
    if !output_path.exists() {
        // Re-copy from store
        let _ = std::fs::copy(&store_bin, output_path);
    }

    Some(store_bin)
}

/// Write build lock after successful compilation.
fn write_build_lock(manifest_path: &Path, output_path: &Path, store_hash: &str) {
    if let Some(manifest_hash) = manifest_content_hash(manifest_path) {
        let lock_path = build_lock_path(output_path);
        let lock = serde_json::json!({
            "manifest_hash": manifest_hash,
            "store_hash": store_hash,
        });
        let _ = std::fs::write(&lock_path, serde_json::to_string_pretty(&lock).unwrap_or_default());
    }
}
