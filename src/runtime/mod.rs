pub mod cache;
pub mod cli;
pub mod config;
mod verify;

use crate::ir::{BesogneIR, ContentId, ResolvedNode, ResolvedNativeNode};
use crate::ir::dag;
use crate::manifest::Phase;
use crate::output::{self, OutputRenderer};
use crate::probe;
use crate::tracer;
use cache::ContextCache;
use cli::DumpMode;
use std::collections::HashMap;
use std::process::ExitCode;
use std::sync::Mutex;
use std::time::Instant;

/// Run a sealed besogne binary by parsing CLI args from argv.
/// This is the entry point for sealed binaries (IR embedded in the executable).
pub fn run(ir: BesogneIR) -> ExitCode {
    let config = cli::RuntimeConfig::from_cli(&ir);
    run_with_config(ir, &config)
}

/// Run a besogne IR with an explicit configuration.
/// This is the programmatic entry point — no CLI parsing involved.
pub fn run_with_config(ir: BesogneIR, config: &cli::RuntimeConfig) -> ExitCode {
    let renderer = output::renderer_for_format(&config.log_format, config.verbose);
    let mut emitter = output::EventEmitter::new(renderer);
    run_with_emitter(ir, config, &mut emitter)
}

/// Run with a custom event handler that receives structured events
/// alongside the built-in renderer.
pub fn run_with_handler(
    ir: BesogneIR,
    config: &cli::RuntimeConfig,
    handler: Box<dyn crate::event::EventHandler>,
) -> ExitCode {
    let renderer = output::renderer_for_format(&config.log_format, config.verbose);
    let mut emitter = output::EventEmitter::with_handler(renderer, handler);
    run_with_emitter(ir, config, &mut emitter)
}

/// Internal: run with a specific emitter (renderer + optional event handler).
fn run_with_emitter(ir: BesogneIR, config: &cli::RuntimeConfig, renderer: &mut output::EventEmitter) -> ExitCode {
    // Handle dump modes (exit early)
    if let Some(dump_mode) = &config.dump {
        return handle_dump(&ir, dump_mode);
    }

    // Compute besogne hash for memoization (needed by both --status and normal run)
    let besogne_hash = compute_besogne_hash(&ir);

    // cd to the manifest's directory — ensures mise/direnv/relative paths work
    if !ir.metadata.workdir.is_empty() {
        if let Err(e) = std::env::set_current_dir(&ir.metadata.workdir) {
            eprintln!("{}", crate::output::style::error_diag(&format!("cannot cd to {}: {e}", ir.metadata.workdir)));
            return ExitCode::from(2);
        }
    }

    let start = Instant::now();

    let flag_vars = config.flag_env.clone();

    let mut context = ContextCache::load(&besogne_hash);

    if config.debug {
        eprintln!("  \x1b[2m[debug] ── IR summary ──\x1b[0m");
        eprintln!("  \x1b[2m[debug] besogne_hash={}\x1b[0m", &besogne_hash[..16.min(besogne_hash.len())]);
        eprintln!("  \x1b[2m[debug] compiler_hash={}\x1b[0m", &context.compiler_hash[..16.min(context.compiler_hash.len())]);
        eprintln!("  \x1b[2m[debug] cache_dir={}\x1b[0m", cache::cache_dir(&context.compiler_hash, &besogne_hash));
        eprintln!("  \x1b[2m[debug] workdir={}\x1b[0m", if ir.metadata.workdir.is_empty() { "." } else { &ir.metadata.workdir });
        // Node counts by type and phase
        let mut type_counts: HashMap<&str, usize> = HashMap::new();
        let mut phase_counts: HashMap<&str, usize> = HashMap::new();
        for node in &ir.nodes {
            let type_name = match &node.node {
                ResolvedNativeNode::Env { .. } => "env",
                ResolvedNativeNode::File { .. } => "file",
                ResolvedNativeNode::Binary { .. } => "binary",
                ResolvedNativeNode::Service { .. } => "service",
                ResolvedNativeNode::Command { .. } => "command",
                ResolvedNativeNode::Platform { .. } => "platform",
                ResolvedNativeNode::Dns { .. } => "dns",
                ResolvedNativeNode::Metric { .. } => "metric",
                ResolvedNativeNode::Source { .. } => "source",
                ResolvedNativeNode::Std { .. } => "std",
                ResolvedNativeNode::Flag { .. } => "flag",
            };
            *type_counts.entry(type_name).or_default() += 1;
            let phase = match node.phase {
                Phase::Build => "build",
                Phase::Seal => "seal",
                Phase::Exec => "exec",
            };
            *phase_counts.entry(phase).or_default() += 1;
        }
        let types: Vec<String> = { let mut v: Vec<_> = type_counts.iter().collect(); v.sort_by_key(|(k, _)| **k); v.iter().map(|(k, v)| format!("{k}={v}")).collect() };
        let phases: Vec<String> = { let mut v: Vec<_> = phase_counts.iter().collect(); v.sort_by_key(|(k, _)| **k); v.iter().map(|(k, v)| format!("{k}={v}")).collect() };
        eprintln!("  \x1b[2m[debug] nodes: {} total [{}] [{}]\x1b[0m", ir.nodes.len(), types.join(", "), phases.join(", "));
        // Binary pinning details
        let binaries: Vec<_> = ir.nodes.iter().filter_map(|n| {
            if let ResolvedNativeNode::Binary { name, resolved_path, resolved_version, binary_hash, .. } = &n.node {
                Some((name.as_str(), resolved_path.as_deref().unwrap_or("?"), resolved_version.as_deref().unwrap_or("?"), binary_hash.as_deref().unwrap_or("?")))
            } else { None }
        }).collect();
        if !binaries.is_empty() {
            eprintln!("  \x1b[2m[debug] ── pinned binaries ──\x1b[0m");
            for (name, path, ver, hash) in &binaries {
                eprintln!("  \x1b[2m[debug]   {name} → {path} v{ver} hash={}\x1b[0m", &hash[..16.min(hash.len())]);
            }
        }
        // Content IDs
        eprintln!("  \x1b[2m[debug] ── content IDs ──\x1b[0m");
        for node in &ir.nodes {
            let phase = match node.phase { Phase::Build => "build", Phase::Seal => "seal", Phase::Exec => "exec" };
            eprintln!("  \x1b[2m[debug]   [{phase}] {} parents=[{}]\x1b[0m",
                node.id.0, node.parents.iter().map(|p| &p.0[..20.min(p.0.len())]).collect::<Vec<_>>().join(", "));
        }
    }

    // --status: unified execution tree + diagnostics, then exit
    if config.status {
        output::views::status::render(&ir, &context);
        return ExitCode::SUCCESS;
    }

    // When launched via `besogne run`, the compiler already displayed build info.
    // Skip the build phase display to avoid redundancy.
    let run_mode = std::env::var("BESOGNE_RUN_MODE").is_ok();

    if !run_mode {
        renderer.on_start(&ir);
    }

    // Partition inputs by phase
    let _build_nodes: Vec<&ResolvedNode> = ir.nodes.iter().filter(|i| i.phase == Phase::Build).collect();
    let pre_nodes: Vec<&ResolvedNode> = ir.nodes.iter().filter(|i| i.phase == Phase::Seal).collect();

    // 2. Pre-phase — check seals
    let warmup_cached = !config.force && pre_nodes.iter().all(|input| {
        if context.get_probe(&input.id.0).is_none() {
            return false;
        }
        // For file inputs, verify the file hasn't changed since caching
        // by re-hashing (cheap for small files, catches content changes)
        match &input.node {
            ResolvedNativeNode::File { path: _, .. } | ResolvedNativeNode::Flag { .. } => {
                let fresh = probe::probe_input_with_flags(&input.node, &config.flag_env);
                if let Some(cached) = context.get_probe(&input.id.0) {
                    fresh.success && fresh.hash == cached.hash
                } else {
                    // Flag with value=false succeeds when not set — still valid for cache
                    fresh.success
                }
            }
            _ => true,
        }
    });

    // Fast path: all probes cached → check if we can skip entirely
    if warmup_cached {
        if config.debug { eprintln!("  \x1b[2m[debug] fast path: all {} seal probes cached\x1b[0m", pre_nodes.len()); }
        let mut all_vars = flag_vars;
        let mut hash_parts = Vec::new();
        for input in &pre_nodes {
            if let Some(cached) = context.get_probe(&input.id.0) {
                if config.debug { eprintln!("  \x1b[2m[debug] seal cached {} → {}\x1b[0m", input.id.0, short_hash(&cached.hash)); }
                all_vars.extend(cached.variables.clone());
                hash_parts.push(cached.hash.clone());
            }
        }
        // Include exec-phase source hashes for cache invalidation
        for node in ir.nodes.iter().filter(|n| n.phase == Phase::Exec && matches!(&n.node, ResolvedNativeNode::Source { .. })) {
            let result = probe::probe_input(&node.node);
            if result.success {
                if config.debug { eprintln!("  \x1b[2m[debug] exec source {} → {}\x1b[0m", node.id.0, &result.hash[..16.min(result.hash.len())]); }
                hash_parts.push(result.hash.clone());
                all_vars.extend(result.variables.clone());
                context.set_probe(node.id.0.clone(), result.hash, result.variables);
            }
        }

        hash_parts.sort();
        let input_hash = blake3::hash(hash_parts.join(":").as_bytes())
            .to_hex()
            .to_string();
        if config.debug { eprintln!("  \x1b[2m[debug] input_hash = {} ({} parts)\x1b[0m", short_hash(&input_hash), hash_parts.len()); }

        let can_skip = !has_side_effects(&ir) && context.can_skip(&input_hash);
        if config.debug {
            if has_side_effects(&ir) {
                eprintln!("  \x1b[2m[debug] has side_effects → cannot skip\x1b[0m");
            } else if can_skip {
                eprintln!("  \x1b[2m[debug] input_hash matches cached → checking backward validity\x1b[0m");
            } else {
                eprintln!("  \x1b[2m[debug] input_hash not in cache → must execute\x1b[0m");
            }
        }
        if can_skip {
            // Backward check: re-probe persistent exec-phase children that
            // have cache entries. If a previously-cached file is now missing
            // (e.g., user deleted node_modules/), fall through to re-execute.
            let outputs_valid = ir.nodes.iter()
                .filter(|n| n.phase == Phase::Exec && n.node.is_persistent())
                .filter(|n| context.get_probe(&n.id.0).is_some())
                .all(|n| {
                    let fresh = probe::probe_input(&n.node);
                    let valid = fresh.success && context.get_probe(&n.id.0)
                        .map_or(false, |c| c.hash == fresh.hash);
                    if config.debug && !valid {
                        let cached_h = context.get_probe(&n.id.0).map(|c| short_hash(&c.hash).to_string()).unwrap_or_else(|| "none".into());
                        let fresh_h = short_hash(&fresh.hash);
                        eprintln!("  \x1b[2m[debug] backward drift: {} cached={} fresh={} ok={}\x1b[0m",
                            &n.id.0[..30.min(n.id.0.len())], cached_h, fresh_h, fresh.success);
                    }
                    valid
                });

            if outputs_valid {
                if config.debug { eprintln!("  \x1b[2m[debug] all outputs valid → full skip\x1b[0m"); }
                let total_nodes = ir.nodes.len();
                if let Some(lr) = context.get_last_run() {
                    renderer.on_skip(total_nodes, &lr.ran_at, lr.duration_ms);
                }
                return ExitCode::SUCCESS;
            }
            if config.debug { eprintln!("  \x1b[2m[debug] persistent output drifted → re-execute\x1b[0m"); }
        }

        // Build phase: never shown by runtime — compiler already showed it.
        // Seal phase: all probes cached → skip display entirely.
        // Go straight to exec DAG.
        return execute_dag(&ir, all_vars, input_hash, renderer, &mut context, start, config.force, config.debug, &std::collections::HashSet::new());
    }

    // Full warmup: probe all seals in parallel
    // Build phase never shown by runtime — compiler already showed it.

    let all_variables = Mutex::new(flag_vars);
    let pre_hashes = Mutex::new(Vec::<String>::new());
    let pre_hard_failed = Mutex::new(false);
    let probe_results = Mutex::new(Vec::new());

    crossbeam::scope(|s| {
        let handles: Vec<_> = pre_nodes
            .iter()
            .map(|input| {
                let all_vars = &all_variables;
                let hashes = &pre_hashes;
                let hard_failed = &pre_hard_failed;
                let results = &probe_results;

                let flag_env = &config.flag_env;
                s.spawn(move |_| {
                    let result = probe::probe_input_with_flags(&input.node, flag_env);
                    results.lock().unwrap().push(((*input).clone(), result.clone()));
                    if result.success {
                        all_vars.lock().unwrap().extend(result.variables.clone());
                        hashes.lock().unwrap().push(result.hash.clone());
                    } else {
                        // Distinguish skip (on_missing=skip) from hard fail
                        let is_skip = is_skip_on_missing(&input.node);
                        if !is_skip {
                            *hard_failed.lock().unwrap() = true;
                        }
                    }
                })
            })
            .collect();
        for h in handles { h.join().unwrap(); }
    })
    .unwrap();

    let mut results = probe_results.into_inner().unwrap();
    results.sort_by(|a, b| a.0.id.0.cmp(&b.0.id.0));

    if config.debug {
        for (input, result) in &results {
            let status = if result.success { "ok" } else { "FAIL" };
            let hash_short = if result.hash.len() >= 16 { short_hash(&result.hash) } else { &result.hash };
            let cached = if context.get_probe(&input.id.0).is_some() { " (cached)" } else { " (fresh)" };
            eprintln!("  \x1b[2m[debug] seal {} → {} {}{}\x1b[0m", input.id.0, status, hash_short, cached);
        }
    }

    // Collect skipped node IDs (on_missing=skip probes that failed)
    let mut skipped_node_ids: std::collections::HashSet<ContentId> = std::collections::HashSet::new();

    // Only show seal phase if there are fresh (non-cached) probes
    let has_fresh_probes = results.iter().any(|(input, _)| {
        context.get_probe(&input.id.0).is_none()
    }) || results.iter().any(|(_, r)| !r.success);

    if has_fresh_probes && !pre_nodes.is_empty() {
        renderer.on_phase_start("seal", pre_nodes.len());
        for (input, result) in &results {
            let status = if result.success {
                output::ProbeStatus::Probed
            } else if is_skip_on_missing(&input.node) {
                output::ProbeStatus::Skipped
            } else {
                output::ProbeStatus::Failed
            };
            renderer.on_probe_result(input, result, status);
        }
        renderer.on_phase_end("seal");
    }

    // Collect skipped node IDs regardless of display
    for (input, result) in &results {
        if !result.success && is_skip_on_missing(&input.node) {
            skipped_node_ids.insert(input.id.clone());
        }
    }

    // Hard failures abort. Skips don't.
    if *pre_hard_failed.lock().unwrap() {
        let wall_ms = start.elapsed().as_millis() as u64;
        renderer.on_summary(2, wall_ms);
        return ExitCode::from(2);
    }

    let mut all_variables = all_variables.into_inner().unwrap();

    let mut hash_parts = pre_hashes.into_inner().unwrap();

    // Probe exec-phase source nodes eagerly so their hashes contribute to
    // input_hash (cache invalidation) and their variables are available to commands.
    let exec_sources: Vec<&ResolvedNode> = ir.nodes.iter()
        .filter(|n| n.phase == Phase::Exec && matches!(&n.node, ResolvedNativeNode::Source { .. }))
        .collect();
    for source in &exec_sources {
        let result = probe::probe_input(&source.node);
        if result.success {
            hash_parts.push(result.hash.clone());
            all_variables.extend(result.variables.clone());
            context.set_probe(source.id.0.clone(), result.hash, result.variables);
        }
    }

    hash_parts.sort();
    let input_hash = blake3::hash(hash_parts.join(":").as_bytes())
        .to_hex()
        .to_string();
    if config.debug { eprintln!("  \x1b[2m[debug] input_hash = {} ({} parts)\x1b[0m", short_hash(&input_hash), hash_parts.len()); }

    if !config.force && !has_side_effects(&ir) && context.can_skip(&input_hash) {
        // Load exec-phase source variables from cache for replay context
        let mut replay_vars = all_variables.clone();
        for node in ir.nodes.iter().filter(|n| n.phase == Phase::Exec) {
            if matches!(&node.node, ResolvedNativeNode::Source { .. }) {
                if let Some(cached) = context.get_probe(&node.id.0) {
                    replay_vars.extend(cached.variables.clone());
                }
            }
        }
        replay_cached_commands(&ir, &context, renderer, &replay_vars);
        let wall_ms = start.elapsed().as_millis() as u64;
        renderer.on_summary(0, wall_ms);
        return ExitCode::SUCCESS;
    }

    // Detect which probes changed (compare fresh vs cached)
    let mut changed: Vec<String> = Vec::new();
    for (input, result) in &results {
        if !result.success { continue; }
        if let Some(cached) = context.get_probe(&input.id.0) {
            if cached.hash != result.hash {
                changed.push(crate::output::input_label(input));
                if config.debug {
                    let label = crate::output::input_label(input);
                    eprintln!("  \x1b[2m[debug] {label}: hash changed {} → {}\x1b[0m",
                        short_hash(&cached.hash), short_hash(&result.hash));
                    // Show variable diffs for env/source nodes
                    if !cached.variables.is_empty() || !result.variables.is_empty() {
                        for (k, v) in &result.variables {
                            match cached.variables.get(k) {
                                Some(old_v) if old_v != v => {
                                    let old_short = if old_v.len() > 60 { format!("{}...", &old_v[..60]) } else { old_v.clone() };
                                    let new_short = if v.len() > 60 { format!("{}...", &v[..60]) } else { v.clone() };
                                    eprintln!("  \x1b[2m[debug]   ~ {k}: {old_short} → {new_short}\x1b[0m");
                                }
                                None => {
                                    let new_short = if v.len() > 60 { format!("{}...", &v[..60]) } else { v.clone() };
                                    eprintln!("  \x1b[2m[debug]   + {k}: {new_short}\x1b[0m");
                                }
                                _ => {} // unchanged
                            }
                        }
                        for k in cached.variables.keys() {
                            if !result.variables.contains_key(k) {
                                eprintln!("  \x1b[2m[debug]   - {k}\x1b[0m");
                            }
                        }
                    }
                }
            }
        } else {
            changed.push(crate::output::input_label(input));
            if config.debug {
                let label = crate::output::input_label(input);
                eprintln!("  \x1b[2m[debug] {label}: new (no cached hash)\x1b[0m");
            }
        }
    }
    if !changed.is_empty() && !config.force {
        renderer.on_changed_probes(&changed);
    }

    // Update cache with fresh probe results
    for (input, result) in &results {
        if result.success {
            context.set_probe(input.id.0.clone(), result.hash.clone(), result.variables.clone());
        }
    }

    execute_dag(&ir, all_variables, input_hash, renderer, &mut context, start, config.force, config.debug, &skipped_node_ids)
}

/// Compute the forward hash for a command: BLAKE3 of its parents' hashes.
/// For command parents → use their child_hash. For probe parents → use probe hash.
/// Also includes the global seal input_hash so seal-phase changes invalidate
/// commands that implicitly depend on seal probes (env vars, file nodes, etc.).
fn compute_parent_hash(
    node: &ResolvedNode,
    ir: &BesogneIR,
    context: &ContextCache,
    seal_input_hash: &str,
) -> String {
    let mut hashes = Vec::new();

    // Always include the seal input hash — any seal-phase change affects all commands
    hashes.push(seal_input_hash.to_string());

    for parent_id in &node.parents {
        if let Some(parent) = ir.nodes.iter().find(|n| n.id == *parent_id) {
            match &parent.node {
                ResolvedNativeNode::Command { name, .. } => {
                    if let Some(cached) = context.get_command(name) {
                        hashes.push(cached.child_hash.clone());
                    }
                }
                _ => {
                    if let Some(cached) = context.get_probe(&parent_id.0) {
                        hashes.push(cached.hash.clone());
                    }
                }
            }
        }
    }
    hashes.sort();
    blake3::hash(hashes.join(":").as_bytes()).to_hex().to_string()
}

/// Compute the backward hash: BLAKE3 of persistent child node hashes.
/// Re-probes persistent children from reality. Std children use cache (ephemeral).
fn compute_child_hash(
    node_id: &ContentId,
    ir: &BesogneIR,
    context: &ContextCache,
) -> (bool, String) {
    let children: Vec<&ResolvedNode> = ir.nodes.iter()
        .filter(|n| n.parents.contains(node_id))
        .collect();

    let mut hashes = Vec::new();
    let mut drifted = false;

    for child in &children {
        if child.node.is_persistent() {
            // Persistent: re-probe from reality
            let fresh = probe::probe_input(&child.node);
            if fresh.success {
                let cached_hash = context.get_probe(&child.id.0).map(|c| c.hash.as_str());
                if cached_hash.is_some() && cached_hash != Some(fresh.hash.as_str()) {
                    drifted = true;
                }
                hashes.push(fresh.hash);
            } else {
                // Persistent child is gone/broken
                drifted = true;
                hashes.push(String::new());
            }
        } else {
            // Ephemeral (std): trust cache — cannot drift externally
            if let Some(cached) = context.get_probe(&child.id.0) {
                hashes.push(cached.hash.clone());
            }
        }
    }

    hashes.sort();
    let hash = blake3::hash(hashes.join(":").as_bytes()).to_hex().to_string();
    (drifted, hash)
}

/// Determine the execution mode for a command based on cache state.
fn determine_command_mode(
    node: &ResolvedNode,
    ir: &BesogneIR,
    context: &ContextCache,
    forced_dirty: bool,
    force_flag: bool,
    seal_input_hash: &str,
    debug: bool,
) -> cache::CommandMode {
    let name_for_debug = match &node.node {
        ResolvedNativeNode::Command { name, .. } => name.as_str(),
        _ => &node.id.0,
    };

    // side_effects → always run
    if let ResolvedNativeNode::Command { side_effects: true, .. } = &node.node {
        if debug { eprintln!("    \x1b[2m[debug] {name_for_debug}: side_effects=true → AlwaysRun\x1b[0m"); }
        return cache::CommandMode::AlwaysRun;
    }

    // --force → full detection
    if force_flag {
        if debug { eprintln!("    \x1b[2m[debug] {name_for_debug}: --force → FullDetection\x1b[0m"); }
        return cache::CommandMode::FullDetection;
    }

    // Forced dirty by upstream propagation
    if forced_dirty {
        if debug { eprintln!("    \x1b[2m[debug] {name_for_debug}: dirty propagation → Detection\x1b[0m"); }
        return cache::CommandMode::Detection;
    }

    // Check if we have a cached entry
    let cmd_name = match &node.node {
        ResolvedNativeNode::Command { name, .. } => name,
        _ => return cache::CommandMode::FullDetection,
    };

    let cached = match context.get_command(cmd_name) {
        Some(c) => c,
        None => {
            if debug { eprintln!("    \x1b[2m[debug] {cmd_name}: no cache entry → FullDetection (first run)\x1b[0m"); }
            return cache::CommandMode::FullDetection;
        }
    };

    // Forward check: parent hash changed?
    let current_parent_hash = compute_parent_hash(node, ir, context, seal_input_hash);
    if debug {
        eprintln!("    \x1b[2m[debug] {cmd_name}: parent_hash cached={} current={}\x1b[0m",
            &cached.parent_hash[..16.min(cached.parent_hash.len())],
            short_hash(&current_parent_hash));
    }
    if cached.parent_hash != current_parent_hash {
        let mode = if context.verified_hash.as_deref() != Some(&current_parent_hash) {
            cache::CommandMode::FullDetection
        } else {
            cache::CommandMode::Detection
        };
        if debug {
            eprintln!("    \x1b[2m[debug] {cmd_name}: parent_hash changed → {mode:?}\x1b[0m");
            // Show which parents contributed to the hash change
            for parent_id in &node.parents {
                if let Some(parent) = ir.nodes.iter().find(|n| n.id == *parent_id) {
                    let (kind, hash) = match &parent.node {
                        ResolvedNativeNode::Command { name, .. } => {
                            let h = context.get_command(name).map(|c| c.child_hash.clone()).unwrap_or_default();
                            ("cmd", h)
                        }
                        _ => {
                            let h = context.get_probe(&parent_id.0).map(|c| c.hash.clone()).unwrap_or_default();
                            ("probe", h)
                        }
                    };
                    let hash_short = if hash.len() >= 16 { short_hash(&hash) } else { &hash };
                    eprintln!("    \x1b[2m[debug]   parent {kind}:{} → {hash_short}\x1b[0m",
                        &parent_id.0[..24.min(parent_id.0.len())]);
                }
            }
        }
        return mode;
    }

    // Backward check: persistent children drifted?
    let (drifted, current_child_hash) = compute_child_hash(&node.id, ir, context);
    if debug {
        eprintln!("    \x1b[2m[debug] {cmd_name}: child_hash={} drifted={drifted}\x1b[0m",
            short_hash(&current_child_hash));
    }
    if drifted {
        if debug {
            eprintln!("    \x1b[2m[debug] {cmd_name}: child drifted → Lightweight\x1b[0m");
            // Show which children drifted
            let children: Vec<&ResolvedNode> = ir.nodes.iter()
                .filter(|n| n.parents.contains(&node.id))
                .collect();
            for child in &children {
                if child.node.is_persistent() {
                    let fresh = probe::probe_input(&child.node);
                    let cached_hash = context.get_probe(&child.id.0).map(|c| c.hash.clone());
                    let fresh_short = short_hash(&fresh.hash);
                    let cached_short = cached_hash.as_ref().map(|h| short_hash(h)).unwrap_or("none");
                    if cached_hash.as_deref() != Some(&fresh.hash) {
                        eprintln!("    \x1b[2m[debug]   drifted child {}: {} → {}\x1b[0m",
                            &child.id.0[..24.min(child.id.0.len())], cached_short, fresh_short);
                    }
                }
            }
        }
        return cache::CommandMode::Lightweight;
    }

    if debug { eprintln!("    \x1b[2m[debug] {cmd_name}: all hashes match → Skip\x1b[0m"); }
    cache::CommandMode::Skip
}

/// Safe short hash: first 16 chars or full string if shorter.
fn short_hash(h: &str) -> &str {
    &h[..16.min(h.len())]
}

/// Apply a single binding with merge strategy.
fn apply_binding(
    env: &mut HashMap<String, String>,
    key: &str,
    value: &str,
    merge: crate::ir::types::EnvMergeResolved,
    separator: &str,
) {
    use crate::ir::types::EnvMergeResolved;
    match merge {
        EnvMergeResolved::Override => {
            env.insert(key.to_string(), value.to_string());
        }
        EnvMergeResolved::Prepend => {
            let existing = env.get(key).cloned().unwrap_or_default();
            if existing.is_empty() {
                env.insert(key.to_string(), value.to_string());
            } else {
                env.insert(key.to_string(), format!("{value}{separator}{existing}"));
            }
        }
        EnvMergeResolved::Append => {
            let existing = env.get(key).cloned().unwrap_or_default();
            if existing.is_empty() {
                env.insert(key.to_string(), value.to_string());
            } else {
                env.insert(key.to_string(), format!("{existing}{separator}{value}"));
            }
        }
        EnvMergeResolved::Fallback => {
            env.entry(key.to_string()).or_insert_with(|| value.to_string());
        }
    }
}

/// Collect DAG-scoped bindings for a node by walking its ancestors.
/// Starts with seal_variables (global), then overlays exec-phase ancestor bindings.
/// Closest ancestor wins (inner scope shadows outer). Merge strategies applied per-env-node.
fn collect_scoped_env(
    node: &ResolvedNode,
    seal_variables: &HashMap<String, String>,
    exec_bindings: &HashMap<ContentId, HashMap<String, String>>,
    ir: &BesogneIR,
) -> HashMap<String, String> {
    let mut env = seal_variables.clone();

    // Walk ancestors in reverse-topological order (furthest first, closest last = closest wins)
    let mut ancestors = Vec::new();
    let mut stack: Vec<&ContentId> = node.parents.iter().collect();
    let mut visited = std::collections::HashSet::new();
    while let Some(pid) = stack.pop() {
        if !visited.insert(pid.clone()) { continue; }
        ancestors.push(pid.clone());
        if let Some(parent) = ir.nodes.iter().find(|n| n.id == *pid) {
            for gpid in &parent.parents {
                stack.push(gpid);
            }
        }
    }
    // Furthest first, closest last
    ancestors.reverse();

    for ancestor_id in &ancestors {
        if let Some(bindings) = exec_bindings.get(ancestor_id) {
            // Look up the IR node to get merge strategy
            let ir_node = ir.nodes.iter().find(|n| n.id == *ancestor_id);
            let (merge, separator) = ir_node
                .and_then(|n| match &n.node {
                    ResolvedNativeNode::Env { merge, separator, .. } => Some((*merge, separator.as_str())),
                    _ => None,
                })
                .unwrap_or((crate::ir::types::EnvMergeResolved::Override, ":"));

            for (k, v) in bindings {
                apply_binding(&mut env, k, v, merge, separator);
            }
        }
    }
    env
}

/// Execute the exec-phase DAG
fn execute_dag(
    ir: &BesogneIR,
    seal_variables: HashMap<String, String>,
    input_hash: String,
    renderer: &mut output::EventEmitter,
    context: &mut ContextCache,
    start: Instant,
    force: bool,
    debug: bool,
    skipped_seal_ids: &std::collections::HashSet<ContentId>,
) -> ExitCode {
    let (graph, _) = match dag::build_exec_dag(ir) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("{}", crate::output::style::error_diag(&e.to_string()));
            return ExitCode::from(2);
        }
    };

    let tiers = match dag::compute_tiers(&graph) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("{}", crate::output::style::error_diag(&e.to_string()));
            return ExitCode::from(2);
        }
    };

    let mut last_exit_code = 0;

    // DAG-scoped bindings: exec-phase nodes store their produced variables here.
    // Commands collect scope by walking ancestors (seal_variables + exec_bindings from ancestors).
    let mut exec_bindings: HashMap<ContentId, HashMap<String, String>> = HashMap::new();

    // Per-command dirty propagation: nodes in this set are forced to re-run
    // because an upstream command's persistent outputs changed.
    let mut dirty_set: std::collections::HashSet<petgraph::graph::NodeIndex> = std::collections::HashSet::new();

    // Skip propagation: exec nodes whose seal-phase parents were skipped (on_missing: skip).
    // Propagates transitively — if a parent is skipped, all descendants are skipped too.
    let mut skip_set: std::collections::HashSet<petgraph::graph::NodeIndex> = {
        let mut set = std::collections::HashSet::new();
        if !skipped_seal_ids.is_empty() {
            // Find exec nodes that have a skipped seal node as a parent
            let input_by_id: HashMap<&ContentId, &ResolvedNode> = ir.nodes.iter()
                .map(|n| (&n.id, n)).collect();
            for node_idx in graph.node_indices() {
                let content_id = &graph[node_idx];
                if let Some(node) = input_by_id.get(content_id) {
                    let has_skipped_parent = node.parents.iter()
                        .any(|pid| skipped_seal_ids.contains(pid));
                    if has_skipped_parent {
                        // BFS: mark this node and all descendants as skipped
                        let mut stack = vec![node_idx];
                        while let Some(idx) = stack.pop() {
                            if set.insert(idx) {
                                for neighbor in graph.neighbors_directed(idx, petgraph::Direction::Outgoing) {
                                    stack.push(neighbor);
                                }
                            }
                        }
                    }
                }
            }
        }
        set
    };

    // Build binary name → resolved path map from build-phase inputs
    let binary_paths: HashMap<String, String> = ir.nodes.iter()
        .filter_map(|i| match &i.node {
            ResolvedNativeNode::Binary { name, resolved_path: Some(path), .. } => {
                Some((name.clone(), path.clone()))
            }
            _ => None,
        })
        .collect();

    // Build binary name → version map
    let binary_versions: HashMap<String, String> = ir.nodes.iter()
        .filter_map(|i| match &i.node {
            ResolvedNativeNode::Binary { name, resolved_version: Some(ver), .. } => {
                Some((name.clone(), ver.clone()))
            }
            _ => None,
        })
        .collect();

    // Build set of secret env var names
    let secret_vars: std::collections::HashSet<String> = ir.nodes.iter()
        .filter_map(|i| match &i.node {
            ResolvedNativeNode::Env { name, secret: true, .. } => Some(name.clone()),
            _ => None,
        })
        .collect();

    // Collect all declared binary names for undeclared dep detection
    let declared_binaries: std::collections::HashSet<String> = ir.nodes.iter()
        .filter_map(|i| match &i.node {
            ResolvedNativeNode::Binary { name, .. } => Some(name.clone()),
            _ => None,
        })
        .collect();

    // Collect all declared env var names
    let declared_env: std::collections::HashSet<String> = ir.nodes.iter()
        .filter_map(|i| match &i.node {
            ResolvedNativeNode::Env { name, .. } => Some(name.clone()),
            _ => None,
        })
        .collect();

    // Static analysis: extract $VAR references from all command run: fields.
    // Only env vars that appear here AND are accessed at runtime will be flagged.
    // This eliminates noise from shell/runtime init reading all env vars.
    let statically_referenced_env: std::collections::HashSet<String> = ir.nodes.iter()
        .filter_map(|i| match &i.node {
            ResolvedNativeNode::Command { run, .. } => Some(run),
            _ => None,
        })
        .flat_map(|run| run.iter().flat_map(|arg| extract_env_refs(arg)))
        .collect();

    let mut undeclared_binaries: std::collections::HashSet<String> = std::collections::HashSet::new();

    // Idempotency verification: on first run, re-run each command (except side_effects)
    // to check whether outputs are deterministic.
    let first_run = force || context.verified_hash.is_none();
    let mut non_idempotent: Vec<String> = Vec::new();

    let input_by_id: HashMap<_, _> = ir
        .nodes
        .iter()
        .filter(|i| i.phase == Phase::Exec)
        .map(|i| (i.id.clone(), i))
        .collect();

    let exec_count = ir.nodes.iter().filter(|n| n.phase == Phase::Exec).count();

    if debug {
        eprintln!("\n  \x1b[2m[debug] ── DAG ({} nodes, {} tiers) ──\x1b[0m", graph.node_count(), tiers.len());
        for (tier_idx, tier) in tiers.iter().enumerate() {
            let names: Vec<String> = tier.iter().map(|&idx| {
                let id = &graph[idx];
                input_by_id.get(id).map(|n| match &n.node {
                    ResolvedNativeNode::Command { name, .. } => format!("cmd:{name}"),
                    ResolvedNativeNode::Std { stream, .. } => format!("std:{stream}"),
                    ResolvedNativeNode::Source { format, .. } => format!("source:{format}"),
                    ResolvedNativeNode::Flag { name, .. } => format!("flag:{name}"),
                    _ => format!("node:{}", &id.0[..20.min(id.0.len())]),
                }).unwrap_or_else(|| format!("?:{}", &id.0[..12.min(id.0.len())]))
            }).collect();
            eprintln!("  \x1b[2m[debug]   tier {tier_idx}: [{}]\x1b[0m", names.join(", "));
        }
        // Show edges
        for edge in graph.edge_indices() {
            if let Some((src, dst)) = graph.edge_endpoints(edge) {
                let src_id = &graph[src].0;
                let dst_id = &graph[dst].0;
                eprintln!("  \x1b[2m[debug]   edge {} → {}\x1b[0m",
                    &src_id[..20.min(src_id.len())], &dst_id[..20.min(dst_id.len())]);
            }
        }
        eprintln!();
    }

    renderer.on_phase_start("exec", exec_count);

    let mut tier_idx = 0;
    for tier in &tiers {
        let tier_start = if debug { Some(Instant::now()) } else { None };
        // ── Phase 1: Collect commands to execute in this tier ──
        struct CmdJob {
            node_idx: petgraph::graph::NodeIndex,
            name: String,
            run: Vec<String>,
            effective_run: Vec<String>,
            cmd_env: HashMap<String, String>,
            workdir: Option<String>,
            side_effects: bool,
            verify: Option<bool>,
            resources: crate::ir::ResourceLimits,
            hide_output: bool,
        }
        let mut jobs: Vec<CmdJob> = Vec::new();

        for &node_idx in tier {
            if skip_set.contains(&node_idx) { continue; }
            let content_id = &graph[node_idx];
            let Some(input) = input_by_id.get(content_id) else { continue };

            if let ResolvedNativeNode::Command {
                name, run, env, side_effects, workdir, force_args, debug_args, verify, resources, hide_output, ..
            } = &input.node {
                if last_exit_code != 0 && !side_effects { continue; }

                let forced_dirty = dirty_set.contains(&node_idx);
                let cmd_mode = determine_command_mode(input, ir, context, forced_dirty, force, &input_hash, debug);
                if debug {
                    eprintln!("    \x1b[2m[debug] {name}: mode={cmd_mode:?} node_id={}\x1b[0m", &content_id.0[..16.min(content_id.0.len())]);
                }

                if cmd_mode == cache::CommandMode::Skip {
                    if let Some(cached) = context.get_command(name) {
                        let mut cmd_env = collect_scoped_env(input, &seal_variables, &exec_bindings, ir);
                        cmd_env.extend(env.clone());
                        let ctx = output::CommandContext {
                            binary_paths: &binary_paths,
                            binary_versions: &binary_versions,
                            env_vars: &cmd_env,
                            secret_vars: &secret_vars,
                        };
                        renderer.on_command_cached(name, run, cached, &ctx);
                    }
                    continue;
                }

                let mut cmd_env = collect_scoped_env(input, &seal_variables, &exec_bindings, ir);
                cmd_env.extend(env.clone());
                let mut effective_run = run.clone();
                if force && !force_args.is_empty() { effective_run.extend(force_args.clone()); }
                if debug && !debug_args.is_empty() { effective_run.extend(debug_args.clone()); }

                if debug {
                    let mut env_keys: Vec<&String> = cmd_env.keys().collect();
                    env_keys.sort();
                    eprintln!("    \x1b[2m[debug] {name}: {} env vars: [{}]\x1b[0m",
                        cmd_env.len(),
                        env_keys.iter().take(20).map(|k| k.as_str()).collect::<Vec<_>>().join(", "));
                    if env_keys.len() > 20 {
                        eprintln!("    \x1b[2m[debug]   ... and {} more\x1b[0m", env_keys.len() - 20);
                    }
                }

                // Show command start header (safe from multiple threads — just eprintln)
                let ctx = output::CommandContext {
                    binary_paths: &binary_paths,
                    binary_versions: &binary_versions,
                    env_vars: &cmd_env,
                    secret_vars: &secret_vars,
                };
                renderer.on_command_start(name, &effective_run, &ctx);

                // Merge resource limits: command-level overrides sandbox defaults
                let effective_resources = crate::ir::ResourceLimits {
                    priority: if resources.priority != crate::ir::PriorityResolved::Normal {
                        resources.priority
                    } else {
                        ir.sandbox.priority
                    },
                    memory_limit: resources.memory_limit.or(ir.sandbox.memory_limit),
                };

                jobs.push(CmdJob {
                    node_idx, name: name.clone(), run: run.clone(),
                    effective_run, cmd_env,
                    workdir: workdir.clone(), side_effects: *side_effects,
                    verify: *verify,
                    resources: effective_resources,
                    hide_output: *hide_output,
                });
            }
        }

        // ── Phase 2: Execute commands in parallel (tracer streams output) ──
        let exec_results: Vec<(CmdJob, Result<tracer::CommandResult, crate::error::BesogneError>)> = if jobs.len() > 1 {
            // Multiple commands in tier → parallel execution with synchronized output
            let sync = tracer::output_sync::OutputSync::new();
            // Register start times for elapsed display in headers
            for job in &jobs {
                sync.register_start(&job.name);
            }
            let flusher = sync.start_flusher();

            let results = crossbeam::scope(|s| {
                let sandbox = &ir.sandbox.env;
                let handles: Vec<_> = jobs.into_iter().map(|job| {
                    let sync = &sync;
                    let cmd_name = job.name.clone();
                    s.spawn(move |_| {
                        tracer::set_hide_output(job.hide_output);
                        let r = tracer::execute_traced_parallel(
                            &job.effective_run, &job.cmd_env, sandbox, job.workdir.as_deref(),
                            sync, &cmd_name, &job.resources);
                        tracer::set_hide_output(false);
                        (job, r)
                    })
                }).collect();
                handles.into_iter().map(|h| h.join().unwrap()).collect::<Vec<_>>()
            }).unwrap();

            flusher.stop();
            results
        } else {
            // Single command or empty → sequential (direct streaming, no sync overhead)
            jobs.into_iter().map(|job| {
                tracer::set_hide_output(job.hide_output);
                let r = tracer::execute_traced(
                    &job.effective_run, &job.cmd_env, &ir.sandbox.env, job.workdir.as_deref(),
                    &job.resources);
                tracer::set_hide_output(false);
                (job, r)
            }).collect()
        };

        // ── Phase 3: Process results sequentially (cache, verify, detect) ──
        for (job, exec_result) in exec_results {
            let content_id = &graph[job.node_idx];
            let input = input_by_id.get(content_id).unwrap();
            let name = &job.name;

            let mut result = match exec_result {
                Ok(r) => r,
                Err(e) => {
                    eprintln!("{}", crate::output::style::error_diag(&e.to_string()));
                    last_exit_code = 126;
                    continue;
                }
            };

            // Use per-process network bytes from preload (accurate) instead of
            // /proc/net/dev diff (system-wide, misleading for per-command attribution).
            // ALWAYS override when preload is active — even if 0 (means no inet traffic).
            if result.preload.is_some() {
                let preload = result.preload.as_ref().unwrap();
                result.net_read_bytes = preload.net_rx_bytes;
                result.net_write_bytes = preload.net_tx_bytes;
            }

            let stdout = String::from_utf8_lossy(&result.stdout).to_string();
            let cmd_stderr = String::from_utf8_lossy(&result.stderr).to_string();

            renderer.on_command_output(name, &stdout, &cmd_stderr);
            renderer.on_command_end(name, &result);

            if debug {
                eprintln!("    \x1b[2m[debug] {name}: exit={} wall={}ms user={}ms sys={}ms rss={}KB\x1b[0m",
                    result.exit_code, result.wall_ms, result.user_ms, result.sys_ms, result.max_rss_kb);
                eprintln!("    \x1b[2m[debug] {name}: disk_r={}B disk_w={}B net_r={}B net_w={}B\x1b[0m",
                    result.disk_read_bytes, result.disk_write_bytes,
                    result.net_read_bytes, result.net_write_bytes);
                if !result.process_tree.is_empty() {
                    eprintln!("    \x1b[2m[debug] {name}: process tree ({} processes):\x1b[0m", result.process_tree.len());
                    for p in &result.process_tree {
                        eprintln!("    \x1b[2m[debug]   pid={} ppid={} {} exit={} wall={}ms user={}ms sys={}ms rss={}KB\x1b[0m",
                            p.pid, p.ppid, p.comm, p.exit_code, p.wall_ms, p.user_ms, p.sys_ms, p.max_rss_kb);
                        if !p.cmdline.is_empty() {
                            eprintln!("    \x1b[2m[debug]     cmdline: {}\x1b[0m",
                                if p.cmdline.len() > 120 { format!("{}...", &p.cmdline[..120]) } else { p.cmdline.clone() });
                        }
                    }
                }
            }

            // Idempotency verification — two strategies:
            //
            // 1. Free check: compare declared outputs (std children, file children,
            //    exit code) between the cached entry and this fresh run.
            //    Two executions with matching declared outputs = idempotent,
            //    no extra re-run needed.
            //
            // 2. Explicit verify: re-run the command a second time and diff.
            //    Only when no cached entry exists for free comparison.
            //
            // verify=true → always explicit, verify=false → never, None → auto.
            const VERIFY_THRESHOLD_MS: u64 = 10_000;
            let mut verified_by_cache = false;

            if !job.side_effects && result.exit_code == 0 && job.verify != Some(false) {
                if let Some(cached) = context.get_command(name) {
                    let outputs_old = verify::collect_declared_outputs(
                        name, content_id, &ir.nodes,
                        &cached.stdout, &cached.stderr, cached.exit_code,
                    );
                    let outputs_new = verify::collect_declared_outputs(
                        name, content_id, &ir.nodes,
                        &stdout, &cmd_stderr, result.exit_code,
                    );
                    let diffs = verify::diff_outputs(&outputs_old, &outputs_new);
                    if diffs.is_empty() {
                        verified_by_cache = true;
                    } else {
                        non_idempotent.push(name.clone());
                        verify::format_verify_human(&verify::VerifyResult {
                            command_name: name.clone(),
                            idempotent: false,
                            diffs,
                        });
                    }
                }
            }

            // Explicit verify: only when free check didn't cover it (no cached entry)
            if !verified_by_cache && !non_idempotent.contains(name) {
                let should_verify = match job.verify {
                    Some(true) => true,
                    Some(false) => false,
                    None => result.wall_ms < VERIFY_THRESHOLD_MS,
                };
                if first_run && !job.side_effects && result.exit_code == 0 && should_verify {
                    eprintln!("    {}", output::style::styled(
                        output::style::diagnostic::VERIFYING,
                        output::style::message::VERIFY_RUN2,
                    ));
                    tracer::set_hide_output(job.hide_output);
                    let vresult = verify::verify_command(
                        name, content_id,
                        &job.effective_run, &job.cmd_env, &ir.sandbox.env, job.workdir.as_deref(),
                        &result, &ir.nodes,
                    );
                    tracer::set_hide_output(false);
                    verify::format_verify_human(&vresult);
                    if !vresult.idempotent {
                        non_idempotent.push(name.clone());
                    }
                }
            }

            // Detect undeclared binaries from process tree
            let run_basenames: std::collections::HashSet<&str> = job.run.iter()
                .filter_map(|a| a.rsplit('/').next())
                .collect();
            for (proc_idx, proc) in result.process_tree.iter().enumerate() {
                if proc.comm.is_empty() { continue; }
                if proc_idx == 0 { continue; }
                let comm = &proc.comm;
                if declared_binaries.contains(comm) { continue; }
                if run_basenames.contains(comm.as_str()) { continue; }
                if ["bash", "sh", "dash", "zsh", "fish", "csh", "tcsh"].contains(&comm.as_str()) { continue; }
                if comm.chars().all(|c| c.is_ascii_hexdigit()) { continue; }
                if proc.cmdline.contains("besogne") { continue; }
                undeclared_binaries.insert(comm.clone());
            }

            // Detect undeclared deps via preload interposer
            if let Some(ref preload) = result.preload {
                let undeclared_env = tracer::preload::find_undeclared_env(
                    &preload.accessed_env, &declared_env, &statically_referenced_env,
                );
                let undeclared_bins = tracer::preload::find_undeclared_binaries(
                    &preload.executed_binaries, &declared_binaries,
                );
                if !undeclared_bins.is_empty() || !undeclared_env.is_empty() {
                    renderer.on_undeclared_deps(&undeclared_bins, &undeclared_env);
                }
            } else if !result.accessed_env.is_empty() {
                let undeclared = tracer::envtrack::find_undeclared(
                    &result.accessed_env, &declared_env, &statically_referenced_env,
                );
                if !undeclared.is_empty() {
                    renderer.on_undeclared_deps(&[], &undeclared);
                }
            }

            // Cache command output (always — std children need stdout/stderr even in debug mode)
            {
                context.set_command(
                    name.clone(),
                    cache::CachedCommand {
                        stdout: stdout.clone(),
                        stderr: cmd_stderr.clone(),
                        exit_code: result.exit_code,
                        wall_ms: result.wall_ms,
                        user_ms: result.user_ms,
                        sys_ms: result.sys_ms,
                        max_rss_kb: result.max_rss_kb,
                        disk_read_bytes: result.disk_read_bytes,
                        disk_write_bytes: result.disk_write_bytes,
                        net_read_bytes: result.net_read_bytes,
                        net_write_bytes: result.net_write_bytes,
                        processes_spawned: result.processes_spawned,
                        process_tree: result.process_tree.clone(),
                        ran_at: chrono::Utc::now().to_rfc3339(),
                        parent_hash: compute_parent_hash(input, ir, context, &input_hash),
                        child_hash: {
                            let (_, ch) = compute_child_hash(content_id, ir, context);
                            ch
                        },
                    },
                );
            }

            // Dirty propagation
            if let Some(old_cached) = context.get_command(name) {
                let (_, new_child_hash) = compute_child_hash(content_id, ir, context);
                if old_cached.child_hash != new_child_hash {
                    if debug {
                        eprintln!("    \x1b[2m[debug] {name}: child_hash changed {} → {} → propagating dirty\x1b[0m",
                            &old_cached.child_hash[..16.min(old_cached.child_hash.len())],
                            short_hash(&new_child_hash));
                    }
                    let mut propagated = Vec::new();
                    for neighbor in graph.neighbors_directed(job.node_idx, petgraph::Direction::Outgoing) {
                        let mut stack = vec![neighbor];
                        while let Some(idx) = stack.pop() {
                            if dirty_set.insert(idx) {
                                propagated.push(graph[idx].0.clone());
                                for next in graph.neighbors_directed(idx, petgraph::Direction::Outgoing) {
                                    stack.push(next);
                                }
                            }
                        }
                    }
                    if debug && !propagated.is_empty() {
                        for p in &propagated {
                            eprintln!("    \x1b[2m[debug]   dirty → {}\x1b[0m", &p[..30.min(p.len())]);
                        }
                    }
                }
            }

            if result.exit_code != 0 {
                last_exit_code = result.exit_code;
            }
        }

        // ── Phase 4: Process non-command nodes sequentially ──
        for &node_idx in tier {
            if skip_set.contains(&node_idx) { continue; }
            let content_id = &graph[node_idx];
            let Some(input) = input_by_id.get(content_id) else { continue };
            if matches!(&input.node, ResolvedNativeNode::Command { .. }) { continue; } // already processed

            match &input.node {

                ResolvedNativeNode::Env { name, on_missing, .. } if input.phase == Phase::Exec => {
                    let result = probe::probe_input(&input.node);
                    if result.success {
                        if !result.variables.is_empty() {
                            exec_bindings.entry(input.id.clone()).or_default().extend(result.variables.clone());
                        }
                        context.set_probe(input.id.0.clone(), result.hash.clone(), result.variables.clone());
                        renderer.on_probe_result(input, &result, output::ProbeStatus::Probed);
                    } else if *on_missing == crate::ir::types::OnMissingResolved::Skip {
                        let mut stack = vec![node_idx];
                        while let Some(idx) = stack.pop() {
                            if skip_set.insert(idx) {
                                for neighbor in graph.neighbors_directed(idx, petgraph::Direction::Outgoing) {
                                    stack.push(neighbor);
                                }
                            }
                        }
                    } else if *on_missing == crate::ir::types::OnMissingResolved::Fail {
                        renderer.on_probe_result(input, &result, output::ProbeStatus::Failed);
                        last_exit_code = 2;
                    }
                }

                ResolvedNativeNode::Flag { name, on_missing, .. } => {
                    let flag_scope = collect_scoped_env(input, &seal_variables, &exec_bindings, ir);
                    let result = probe::probe_input_with_flags(&input.node, &flag_scope);
                    if result.success {
                        if debug { eprintln!("    \x1b[2m[debug] flag '{name}': matched → children execute\x1b[0m"); }
                        exec_bindings.entry(input.id.clone()).or_default().extend(result.variables.clone());
                        context.set_probe(input.id.0.clone(), result.hash.clone(), result.variables);
                    } else if *on_missing == crate::ir::types::OnMissingResolved::Skip {
                        if debug { eprintln!("    \x1b[2m[debug] flag '{name}': not matched → pruning subtree\x1b[0m"); }
                        // BFS: skip this node and all descendants
                        let mut stack = vec![node_idx];
                        while let Some(idx) = stack.pop() {
                            if skip_set.insert(idx) {
                                for neighbor in graph.neighbors_directed(idx, petgraph::Direction::Outgoing) {
                                    stack.push(neighbor);
                                }
                            }
                        }
                    } else {
                        eprintln!("  {} flag '--{name}' not matched",
                            output::style::styled(output::style::status::FAILED, output::style::label::FAILED));
                        last_exit_code = 2;
                    }
                }

                ResolvedNativeNode::Service { .. }
                | ResolvedNativeNode::Dns { .. }
                | ResolvedNativeNode::Metric { .. } => {
                    let result = probe::probe_input(&input.node);
                    let status = if result.success {
                        output::ProbeStatus::Probed
                    } else {
                        output::ProbeStatus::Failed
                    };
                    renderer.on_probe_result(input, &result, status);
                    if !result.success {
                        last_exit_code = 1;
                    }
                }

                ResolvedNativeNode::Std { stream, contains, expect, .. } => {
                    if debug { eprintln!("    \x1b[2m[debug] std:{stream} node_id={}\x1b[0m", &input.id.0[..16.min(input.id.0.len())]); }
                    // Find parent command's cached output
                    let parent_cmd_name = input.parents.iter().find_map(|pid| {
                        input_by_id.get(pid).and_then(|p| {
                            if let ResolvedNativeNode::Command { name, .. } = &p.node {
                                Some(name.clone())
                            } else { None }
                        })
                    });

                    let (content, label) = if let Some(cmd_name) = &parent_cmd_name {
                        if let Some(cached) = context.get_command(cmd_name) {
                            match stream.as_str() {
                                "stdout" => (cached.stdout.clone(), format!("std:stdout of {cmd_name}")),
                                "stderr" => (cached.stderr.clone(), format!("std:stderr of {cmd_name}")),
                                "exit_code" => (cached.exit_code.to_string(), format!("std:exit_code of {cmd_name}")),
                                _ => (String::new(), format!("std:{stream} of {cmd_name}")),
                            }
                        } else {
                            (String::new(), format!("std:{stream} (parent not cached)"))
                        }
                    } else {
                        (String::new(), format!("std:{stream} (no parent command)"))
                    };

                    let mut valid = true;
                    if let Some(expected) = expect {
                        if content.trim() != expected.as_str() {
                            eprintln!("  {} {label}: expected {expected}, got {}",
                                output::style::styled(output::style::status::FAILED, output::style::label::FAILED),
                                content.trim());
                            valid = false;
                        }
                    }
                    for pattern in contains {
                        if !content.contains(pattern.as_str()) {
                            eprintln!("  {} {label}: expected to contain \"{pattern}\"",
                                output::style::styled(output::style::status::FAILED, output::style::label::FAILED));
                            valid = false;
                        }
                    }

                    if valid {
                        let hash = blake3::hash(content.as_bytes()).to_hex().to_string();
                        if debug { eprintln!("    \x1b[2m[debug] std:{stream} hash={} content_len={}\x1b[0m", short_hash(&hash), content.len()); }
                        context.set_probe(input.id.0.clone(), hash, HashMap::new());
                        eprintln!("  {} {label}",
                            output::style::styled(output::style::status::PROBED, output::style::label::PROBED));
                    } else {
                        last_exit_code = 3;
                    }
                }

                _ => {
                    // Source nodes with a std parent: parse the parent command's stdout
                    if let ResolvedNativeNode::Source { format, select, .. } = &input.node {
                        if debug { eprintln!("    \x1b[2m[debug] source node format={format} id={}\x1b[0m", &input.id.0[..16.min(input.id.0.len())]); }
                        // Find std parent → command parent → cached stdout
                        let std_content = input.parents.iter().find_map(|pid| {
                            let std_node = input_by_id.get(pid)?;
                            if let ResolvedNativeNode::Std { stream, .. } = &std_node.node {
                                // Find the command that owns this std node
                                let cmd_name = std_node.parents.iter().find_map(|cpid| {
                                    let cmd_node = input_by_id.get(cpid)?;
                                    if let ResolvedNativeNode::Command { name, .. } = &cmd_node.node {
                                        Some(name.clone())
                                    } else { None }
                                })?;
                                let cached = context.get_command(&cmd_name)?;
                                let content = match stream.as_str() {
                                    "stdout" => cached.stdout.clone(),
                                    "stderr" => cached.stderr.clone(),
                                    _ => return None,
                                };
                                Some(content)
                            } else { None }
                        });

                        if let Some(content) = std_content {
                            if debug { eprintln!("    \x1b[2m[debug] source: parsing {} bytes of {format} from std parent\x1b[0m", content.len()); }
                            match crate::probe::source::parse_env_map(format, &content) {
                                Ok(env) => {
                                    let filtered = match select {
                                        Some(keys) => env.into_iter()
                                            .filter(|(k, _)| keys.contains(k))
                                            .collect(),
                                        None => env,
                                    };
                                    if debug { eprintln!("    \x1b[2m[debug] source: injecting {} env vars (DAG-scoped)\x1b[0m", filtered.len()); }
                                    exec_bindings.entry(input.id.clone()).or_default().extend(filtered);
                                }
                                Err(e) => {
                                    eprintln!("  {} source: {e}",
                                        output::style::styled(output::style::status::FAILED, output::style::label::FAILED));
                                    last_exit_code = 3;
                                }
                            }
                        }
                        // File-based sources were already probed eagerly
                        continue;
                    }
                    let result = probe::probe_input(&input.node);
                    if result.success {
                        // Store probe variables in DAG-scoped bindings for downstream commands
                        if !result.variables.is_empty() {
                            exec_bindings.entry(input.id.clone()).or_default().extend(result.variables.clone());
                        }
                        // Probe succeeded — cache the hash
                        context.set_probe(input.id.0.clone(), result.hash.clone(), result.variables.clone());
                        renderer.on_probe_result(input, &result, output::ProbeStatus::Probed);
                    } else {
                        // Probe failed — check if this is a postcondition (has command parent)
                        let is_postcondition = input.parents.iter().any(|pid| {
                            input_by_id.get(pid).map_or(false, |p|
                                matches!(&p.node, ResolvedNativeNode::Command { .. }))
                        });
                        if is_postcondition && context.get_probe(&input.id.0).is_none() {
                            // First run: postcondition file not yet created — skip silently
                            // (parent command should have created it; if it didn't, that's
                            // a command failure, not a probe failure)
                        } else {
                            renderer.on_probe_result(input, &result, output::ProbeStatus::Failed);
                            last_exit_code = 2;
                        }
                    }
                }
            }
        }
        if let Some(ts) = tier_start {
            eprintln!("    \x1b[2m[debug] tier {tier_idx}: {}ms\x1b[0m", ts.elapsed().as_millis());
        }
        tier_idx += 1;
    }

    // Report undeclared dependencies and poison cache if found
    let undeclared_bins: Vec<String> = undeclared_binaries.into_iter().collect();
    if debug && !undeclared_bins.is_empty() {
        eprintln!("  \x1b[2m[debug] ── undeclared dependencies ──\x1b[0m");
        for bin in &undeclared_bins {
            eprintln!("  \x1b[2m[debug]   binary: {bin}\x1b[0m");
        }
    }
    if !undeclared_bins.is_empty() {
        renderer.on_undeclared_deps(&undeclared_bins, &[]);
    }

    // Cache hit/miss stats
    if debug {
        let cmd_count = ir.nodes.iter().filter(|n| matches!(&n.node, ResolvedNativeNode::Command { .. }) && n.phase == Phase::Exec).count();
        let probe_count = context.warmup.len();
        let cmd_cached = context.commands.len();
        eprintln!("\n  \x1b[2m[debug] ── cache stats ──\x1b[0m");
        eprintln!("  \x1b[2m[debug] commands: {cmd_count} total, {cmd_cached} cached\x1b[0m");
        eprintln!("  \x1b[2m[debug] probes: {probe_count} cached\x1b[0m");
        eprintln!("  \x1b[2m[debug] first_run={first_run} non_idempotent=[{}]\x1b[0m", non_idempotent.join(", "));
        eprintln!("  \x1b[2m[debug] verified_hash={}\x1b[0m",
            context.verified_hash.as_deref().map(|h| &h[..16.min(h.len())]).unwrap_or("none"));
    }

    // Store idempotency verification results
    if first_run && !debug {
        context.verified_hash = Some(input_hash.clone());
        context.non_idempotent = non_idempotent.clone();
        if !non_idempotent.is_empty() {
            eprintln!("\n  {} {}",
                output::style::styled(output::style::diagnostic::NOT_IDEMPOTENT, output::style::message::NOT_IDEMPOTENT),
                output::style::dim(&non_idempotent.join(", ")));
            eprintln!("  {}", output::style::dim("add side_effects = true to these commands if intentional"));
        }
    }

    let wall_ms = start.elapsed().as_millis() as u64;
    if !debug {
        if undeclared_bins.is_empty() {
            context.set_last_run(input_hash, last_exit_code, wall_ms);
        } else {
            context.set_last_run("__undeclared_deps__".to_string(), last_exit_code, wall_ms);
        }
        let _ = context.save();
    }
    renderer.on_phase_end("exec");
    renderer.on_summary(last_exit_code, wall_ms);
    ExitCode::from(last_exit_code as u8)
}

/// Execute a command with optional retry logic.
#[allow(dead_code)]
fn execute_command_with_retry(
    name: &str,
    run: &[String],
    env: &HashMap<String, String>,
    sandbox_env: &crate::ir::EnvSandboxResolved,
    workdir: Option<&str>,
    retry: Option<&crate::ir::RetryResolved>,
    _renderer: &mut output::EventEmitter,
    resources: &crate::ir::ResourceLimits,
) -> (Option<tracer::CommandResult>, String, String) {
    let max_attempts = retry.map(|r| r.attempts).unwrap_or(1);
    let deadline = retry.and_then(|r| r.timeout_ms.map(|t| {
        std::time::Instant::now() + std::time::Duration::from_millis(t)
    }));

    let mut last_result = None;
    let mut last_stdout = String::new();
    let mut last_stderr = String::new();

    for attempt in 0..max_attempts {
        if let Some(dl) = deadline {
            if std::time::Instant::now() >= dl {
                eprintln!("  {} retry timeout for '{name}' after {attempt} attempts",
                    output::style::styled(output::style::status::FAILED, output::style::label::FAILED));
                break;
            }
        }

        match tracer::execute_traced(run, env, sandbox_env, workdir, resources) {
            Ok(result) => {
                let stdout = String::from_utf8_lossy(&result.stdout).to_string();
                let stderr = String::from_utf8_lossy(&result.stderr).to_string();

                if result.exit_code == 0 || retry.is_none() || attempt + 1 >= max_attempts {
                    return (Some(result), stdout, stderr);
                }

                last_stdout = stdout;
                last_stderr = stderr;
                last_result = Some(result);
            }
            Err(e) => {
                eprintln!("{}", crate::output::style::error_diag(&e.to_string()));
                if retry.is_none() || attempt + 1 >= max_attempts {
                    return (None, String::new(), String::new());
                }
            }
        }

        if let Some(r) = retry {
            if attempt + 1 < max_attempts {
                let delay = r.delay_for_attempt(attempt);
                let delay = if let Some(dl) = deadline {
                    delay.min(dl.saturating_duration_since(std::time::Instant::now()))
                } else {
                    delay
                };

                if !delay.is_zero() {
                    let ms = delay.as_millis();
                    let dur_str = if ms < 1000 { format!("{}ms", ms) }
                        else if ms < 60_000 { format!("{:.1}s", ms as f64 / 1000.0) }
                        else { format!("{:.1}m", ms as f64 / 60_000.0) };
                    eprintln!("  {} '{name}' retry {}/{} in {dur_str}",
                        output::style::styled(output::style::status::PENDING, "retry"),
                        attempt + 1, max_attempts);
                    std::thread::sleep(delay);
                }
            }
        }
    }

    match last_result {
        Some(r) => (Some(r), last_stdout, last_stderr),
        None => (None, String::new(), String::new()),
    }
}

fn handle_dump(ir: &BesogneIR, mode: &DumpMode) -> ExitCode {
    match mode {
        DumpMode::Internal => {
            let json = serde_json::to_string_pretty(ir)
                .unwrap_or_else(|e| format!("{{\"error\": \"{e}\"}}"));
            println!("{json}");
        }
        DumpMode::Human => {
            println!("{} v{}", ir.metadata.name, ir.metadata.version);
            println!("{}", ir.metadata.description);
            println!("Compiler: {}", cache::compiler_self_hash());
            println!();

            let build_nodes: Vec<_> = ir.nodes.iter().filter(|i| i.phase == Phase::Build).collect();
            let pre_nodes: Vec<_> = ir.nodes.iter().filter(|i| i.phase == Phase::Seal).collect();
            let exec_nodes: Vec<_> = ir.nodes.iter().filter(|i| i.phase == Phase::Exec).collect();

            if !build_nodes.is_empty() {
                println!("Sealed (build phase) ({}):", build_nodes.len());
                for i in &build_nodes { println!("  {}", i.id); }
                println!();
            }
            if !pre_nodes.is_empty() {
                println!("Preconditions (seal phase) ({}):", pre_nodes.len());
                for i in &pre_nodes { println!("  {}", i.id); }
                println!();
            }
            if !exec_nodes.is_empty() {
                println!("Execution (exec phase) ({}):", exec_nodes.len());
                for i in &exec_nodes { println!("  {}", i.id); }
                println!();
            }

            let se_count = ir.nodes.iter().filter(|i| matches!(&i.node,
                ResolvedNativeNode::Command { side_effects: true, .. }
            )).count();
            if se_count > 0 {
                println!("Side effects: {se_count} command(s) always run");
            }
        }
    }
    ExitCode::SUCCESS
}

/// Replay cached command output for all exec-phase commands (on skip)
fn replay_cached_commands(
    ir: &BesogneIR,
    context: &ContextCache,
    renderer: &mut output::EventEmitter,
    all_variables: &HashMap<String, String>,
) {
    // Build context maps (same as execute_dag)
    let binary_paths: HashMap<String, String> = ir.nodes.iter()
        .filter_map(|i| match &i.node {
            ResolvedNativeNode::Binary { name, resolved_path: Some(path), .. } => {
                Some((name.clone(), path.clone()))
            }
            _ => None,
        })
        .collect();
    let binary_versions: HashMap<String, String> = ir.nodes.iter()
        .filter_map(|i| match &i.node {
            ResolvedNativeNode::Binary { name, resolved_version: Some(ver), .. } => {
                Some((name.clone(), ver.clone()))
            }
            _ => None,
        })
        .collect();
    let secret_vars: std::collections::HashSet<String> = ir.nodes.iter()
        .filter_map(|i| match &i.node {
            ResolvedNativeNode::Env { name, secret: true, .. } => Some(name.clone()),
            _ => None,
        })
        .collect();

    for input in &ir.nodes {
        if input.phase != Phase::Exec {
            continue;
        }
        if let ResolvedNativeNode::Command { name, run, env, .. } = &input.node {
            if let Some(cached) = context.get_command(name) {
                // In replay mode, exec bindings aren't available — use seal vars only
                let empty_exec = HashMap::new();
                let mut cmd_env = collect_scoped_env(input, all_variables, &empty_exec, ir);
                cmd_env.extend(env.clone());
                let ctx = output::CommandContext {
                    binary_paths: &binary_paths,
                    binary_versions: &binary_versions,
                    env_vars: &cmd_env,
                    secret_vars: &secret_vars,
                };
                renderer.on_command_cached(name, run, cached, &ctx);
            }
        }
    }
}

/// Returns true if any exec-phase command has side_effects = true
fn has_side_effects(ir: &BesogneIR) -> bool {
    ir.nodes.iter().any(|i| matches!(&i.node,
        ResolvedNativeNode::Command { side_effects: true, .. }
    ))
}

/// Check if a node has on_missing: skip (meaning probe failure = skip, not abort).
fn is_skip_on_missing(node: &ResolvedNativeNode) -> bool {
    matches!(node,
        ResolvedNativeNode::Env { on_missing: crate::ir::types::OnMissingResolved::Skip, .. } |
        ResolvedNativeNode::Flag { on_missing: crate::ir::types::OnMissingResolved::Skip, .. }
    )
}

fn compute_besogne_hash(ir: &BesogneIR) -> String {
    let content = serde_json::to_string(ir).unwrap_or_default();
    blake3::hash(content.as_bytes()).to_hex()[..16].to_string()
}

/// Extract $VAR and ${VAR} references from a string (command arg, script body).
/// Returns variable names only — no $, no braces, no special vars ($?, $$, $0, etc.).
fn extract_env_refs(s: &str) -> Vec<String> {
    let mut vars = Vec::new();
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '$' {
            if chars.peek() == Some(&'{') {
                chars.next();
                let var: String = chars.by_ref()
                    .take_while(|&c| c != '}')
                    .collect();
                // Skip parameter expansion operators: ${VAR:-default}, ${VAR%%pattern}, etc.
                let name = var.split(|c: char| !c.is_alphanumeric() && c != '_')
                    .next()
                    .unwrap_or("");
                if !name.is_empty() && name.chars().next().map(|c| c.is_alphabetic() || c == '_').unwrap_or(false) {
                    vars.push(name.to_string());
                }
            } else {
                let var: String = chars.by_ref()
                    .take_while(|c| c.is_alphanumeric() || *c == '_')
                    .collect();
                if !var.is_empty() && var.chars().next().map(|c| c.is_alphabetic() || c == '_').unwrap_or(false) {
                    vars.push(var);
                }
            }
        }
    }
    vars
}

/// Replace the current process with a besogne binary.
/// Sets `BESOGNE_RUN_MODE=1` so the binary skips build phase display.
/// Returns the IO error if exec fails (on Unix, this function doesn't return on success).
#[cfg(unix)]
pub fn exec_binary(path: &std::path::Path, args: &[String]) -> std::io::Error {
    use std::os::unix::process::CommandExt;
    std::process::Command::new(path)
        .args(args)
        .env("BESOGNE_RUN_MODE", "1")
        .exec()
}

#[cfg(not(unix))]
pub fn exec_binary(path: &std::path::Path, args: &[String]) -> std::io::Error {
    match std::process::Command::new(path)
        .args(args)
        .env("BESOGNE_RUN_MODE", "1")
        .status()
    {
        Ok(status) => std::process::exit(status.code().unwrap_or(1)),
        Err(e) => e,
    }
}
