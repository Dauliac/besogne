pub mod cache;
pub mod cli;
pub mod config;
mod verify;

use crate::ir::{BesogneIR, ContentId, ResolvedNode, ResolvedNativeNode};
use crate::ir::dag;
use crate::manifest::Phase;
use crate::output;
use crate::probe;
use crate::tracer;
use cache::ContextCache;
use cli::{DumpMode, RunMode};
use std::collections::HashMap;
use std::process::ExitCode;
use std::sync::Mutex;
use std::time::Instant;

/// Run a sealed besogne binary
pub fn run(ir: BesogneIR) -> ExitCode {
    let args = cli::parse_runtime_args(&ir);

    // Handle dump modes (exit early)
    if let Some(dump_mode) = &args.dump {
        return handle_dump(&ir, dump_mode);
    }

    // Compute besogne hash for memoization (needed by both --status and normal run)
    let besogne_hash = compute_besogne_hash(&ir);

    let mut renderer = output::renderer_for_format(&args.log_format, args.verbose);

    // cd to the manifest's directory — ensures mise/direnv/relative paths work
    if !ir.metadata.workdir.is_empty() {
        if let Err(e) = std::env::set_current_dir(&ir.metadata.workdir) {
            eprintln!("{}", crate::output::style::error_diag(&format!("cannot cd to {}: {e}", ir.metadata.workdir)));
            return ExitCode::from(2);
        }
    }

    let start = Instant::now();

    let flag_vars = args.flag_env;

    let mut context = ContextCache::load(&besogne_hash);

    // --status: unified execution tree + diagnostics, then exit
    if args.status {
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
    let build_nodes: Vec<&ResolvedNode> = ir.nodes.iter().filter(|i| i.phase == Phase::Build).collect();
    let pre_nodes: Vec<&ResolvedNode> = ir.nodes.iter().filter(|i| i.phase == Phase::Seal).collect();

    // 2. Pre-phase — check seals
    let warmup_cached = !args.force && pre_nodes.iter().all(|input| {
        if context.get_probe(&input.id.0).is_none() {
            return false;
        }
        // For file inputs, verify the file hasn't changed since caching
        // by re-hashing (cheap for small files, catches content changes)
        match &input.node {
            ResolvedNativeNode::File { path, .. } => {
                let fresh = probe::probe_input(&input.node);
                if let Some(cached) = context.get_probe(&input.id.0) {
                    fresh.success && fresh.hash == cached.hash
                } else {
                    false
                }
            }
            _ => true,
        }
    });

    // Fast path: all probes cached → check if we can skip entirely
    if warmup_cached {
        let mut all_vars = flag_vars;
        let mut hash_parts = Vec::new();
        for input in &pre_nodes {
            if let Some(cached) = context.get_probe(&input.id.0) {
                all_vars.extend(cached.variables.clone());
                hash_parts.push(cached.hash.clone());
            }
        }
        // Include exec-phase source hashes for cache invalidation
        for node in ir.nodes.iter().filter(|n| n.phase == Phase::Exec && matches!(&n.node, ResolvedNativeNode::Source { .. })) {
            let result = probe::probe_input(&node.node);
            if result.success {
                hash_parts.push(result.hash.clone());
                all_vars.extend(result.variables.clone());
                context.set_probe(node.id.0.clone(), result.hash, result.variables);
            }
        }

        hash_parts.sort();
        let input_hash = blake3::hash(hash_parts.join(":").as_bytes())
            .to_hex()
            .to_string();

        if !has_side_effects(&ir) && context.can_skip(&input_hash) {
            // Backward check: re-probe persistent exec-phase children that
            // have cache entries. If a previously-cached file is now missing
            // (e.g., user deleted node_modules/), fall through to re-execute.
            let outputs_valid = ir.nodes.iter()
                .filter(|n| n.phase == Phase::Exec && n.node.is_persistent())
                .filter(|n| context.get_probe(&n.id.0).is_some())
                .all(|n| {
                    let fresh = probe::probe_input(&n.node);
                    fresh.success && context.get_probe(&n.id.0)
                        .map_or(false, |c| c.hash == fresh.hash)
                });

            if outputs_valid {
                let total_nodes = ir.nodes.len();
                if let Some(lr) = context.get_last_run() {
                    renderer.on_skip(total_nodes, &lr.ran_at, lr.duration_ms);
                }
                return ExitCode::SUCCESS;
            }
            // Persistent output drifted → fall through to re-execute
        }

        // Build phase: never shown by runtime — compiler already showed it.
        // Seal phase: all probes cached → skip display entirely.
        // Go straight to exec DAG.
        return execute_dag(&ir, all_vars, input_hash, &mut *renderer, &mut context, start, args.force, args.debug, &std::collections::HashSet::new());
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

                s.spawn(move |_| {
                    let result = probe::probe_input(&input.node);
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

    if !args.force && !has_side_effects(&ir) && context.can_skip(&input_hash) {
        // Load exec-phase source variables from cache for replay context
        let mut replay_vars = all_variables.clone();
        for node in ir.nodes.iter().filter(|n| n.phase == Phase::Exec) {
            if matches!(&node.node, ResolvedNativeNode::Source { .. }) {
                if let Some(cached) = context.get_probe(&node.id.0) {
                    replay_vars.extend(cached.variables.clone());
                }
            }
        }
        replay_cached_commands(&ir, &context, &mut *renderer, &replay_vars);
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
            }
        } else {
            changed.push(crate::output::input_label(input));
        }
    }
    if !changed.is_empty() && !args.force {
        renderer.on_changed_probes(&changed);
    }

    // Update cache with fresh probe results
    for (input, result) in &results {
        if result.success {
            context.set_probe(input.id.0.clone(), result.hash.clone(), result.variables.clone());
        }
    }

    execute_dag(&ir, all_variables, input_hash, &mut *renderer, &mut context, start, args.force, args.debug, &skipped_node_ids)
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
) -> cache::CommandMode {
    // side_effects → always run
    if let ResolvedNativeNode::Command { side_effects: true, .. } = &node.node {
        return cache::CommandMode::AlwaysRun;
    }

    // --force → full detection
    if force_flag {
        return cache::CommandMode::FullDetection;
    }

    // Forced dirty by upstream propagation
    if forced_dirty {
        return cache::CommandMode::Detection;
    }

    // Check if we have a cached entry
    let cmd_name = match &node.node {
        ResolvedNativeNode::Command { name, .. } => name,
        _ => return cache::CommandMode::FullDetection, // non-command in DAG
    };

    let cached = match context.get_command(cmd_name) {
        Some(c) => c,
        None => return cache::CommandMode::FullDetection, // first run
    };

    // Forward check: parent hash changed?
    let current_parent_hash = compute_parent_hash(node, ir, context, seal_input_hash);
    if cached.parent_hash != current_parent_hash {
        // Inputs changed — need detection. Also re-verify if never verified for this hash.
        return if context.verified_hash.as_deref() != Some(&current_parent_hash) {
            cache::CommandMode::FullDetection
        } else {
            cache::CommandMode::Detection
        };
    }

    // Backward check: persistent children drifted?
    let (drifted, _current_child_hash) = compute_child_hash(&node.id, ir, context);
    if drifted {
        return cache::CommandMode::Lightweight;
    }

    cache::CommandMode::Skip
}

/// Execute the exec-phase DAG
fn execute_dag(
    ir: &BesogneIR,
    mut all_variables: HashMap<String, String>,
    input_hash: String,
    renderer: &mut dyn output::OutputRenderer,
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

    // Per-command dirty propagation: nodes in this set are forced to re-run
    // because an upstream command's persistent outputs changed.
    let mut dirty_set: std::collections::HashSet<petgraph::graph::NodeIndex> = std::collections::HashSet::new();

    // Skip propagation: exec nodes whose seal-phase parents were skipped (on_missing: skip).
    // Propagates transitively — if a parent is skipped, all descendants are skipped too.
    let skip_set: std::collections::HashSet<petgraph::graph::NodeIndex> = {
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
    renderer.on_phase_start("exec", exec_count);

    for tier in &tiers {
        // ── Phase 1: Collect commands to execute in this tier ──
        struct CmdJob {
            node_idx: petgraph::graph::NodeIndex,
            name: String,
            run: Vec<String>,
            effective_run: Vec<String>,
            cmd_env: HashMap<String, String>,
            workdir: Option<String>,
            side_effects: bool,
        }
        let mut jobs: Vec<CmdJob> = Vec::new();

        for &node_idx in tier {
            if skip_set.contains(&node_idx) { continue; }
            let content_id = &graph[node_idx];
            let Some(input) = input_by_id.get(content_id) else { continue };

            if let ResolvedNativeNode::Command {
                name, run, env, side_effects, workdir, force_args, debug_args, ..
            } = &input.node {
                if last_exit_code != 0 && !side_effects { continue; }

                let forced_dirty = dirty_set.contains(&node_idx);
                let cmd_mode = determine_command_mode(input, ir, context, forced_dirty, force, &input_hash);

                if cmd_mode == cache::CommandMode::Skip {
                    if let Some(cached) = context.get_command(name) {
                        let mut cmd_env = all_variables.clone();
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

                let mut cmd_env = all_variables.clone();
                cmd_env.extend(env.clone());
                let mut effective_run = run.clone();
                if force && !force_args.is_empty() { effective_run.extend(force_args.clone()); }
                if debug && !debug_args.is_empty() { effective_run.extend(debug_args.clone()); }

                // Show command start header (safe from multiple threads — just eprintln)
                let ctx = output::CommandContext {
                    binary_paths: &binary_paths,
                    binary_versions: &binary_versions,
                    env_vars: &cmd_env,
                    secret_vars: &secret_vars,
                };
                renderer.on_command_start(name, &effective_run, &ctx);

                jobs.push(CmdJob {
                    node_idx, name: name.clone(), run: run.clone(),
                    effective_run, cmd_env,
                    workdir: workdir.clone(), side_effects: *side_effects,
                });
            }
        }

        // ── Phase 2: Execute commands in parallel (tracer streams output) ──
        let exec_results: Vec<(CmdJob, Result<tracer::CommandResult, String>)> = if jobs.len() > 1 {
            // Multiple commands in tier → parallel execution with synchronized output
            let sync = tracer::output_sync::OutputSync::new();
            let flusher = sync.start_flusher();

            let results = crossbeam::scope(|s| {
                let sandbox = &ir.sandbox.env;
                let handles: Vec<_> = jobs.into_iter().map(|job| {
                    let sync = &sync;
                    let cmd_name = job.name.clone();
                    s.spawn(move |_| {
                        let r = tracer::execute_traced_parallel(
                            &job.effective_run, &job.cmd_env, sandbox, job.workdir.as_deref(),
                            sync, &cmd_name);
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
                let r = tracer::execute_traced(
                    &job.effective_run, &job.cmd_env, &ir.sandbox.env, job.workdir.as_deref());
                (job, r)
            }).collect()
        };

        // ── Phase 3: Process results sequentially (cache, verify, detect) ──
        for (job, exec_result) in exec_results {
            let content_id = &graph[job.node_idx];
            let input = input_by_id.get(content_id).unwrap();
            let name = &job.name;

            let result = match exec_result {
                Ok(r) => r,
                Err(e) => {
                    eprintln!("{}", crate::output::style::error_diag(&e));
                    last_exit_code = 126;
                    continue;
                }
            };

            let stdout = String::from_utf8_lossy(&result.stdout).to_string();
            let cmd_stderr = String::from_utf8_lossy(&result.stderr).to_string();

            renderer.on_command_output(name, &stdout, &cmd_stderr);
            renderer.on_command_end(name, &result);

            // Idempotency verification
            if first_run && !job.side_effects && result.exit_code == 0 {
                eprintln!("    {}", output::style::styled(
                    output::style::diagnostic::VERIFYING,
                    output::style::message::VERIFY_RUN2,
                ));
                let vresult = verify::verify_command(
                    name, content_id,
                    &job.effective_run, &job.cmd_env, &ir.sandbox.env, job.workdir.as_deref(),
                    &result, &ir.nodes,
                );
                verify::format_verify_human(&vresult);
                if !vresult.idempotent {
                    non_idempotent.push(name.clone());
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

            // Cache command output
            if !debug {
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
                    for neighbor in graph.neighbors_directed(job.node_idx, petgraph::Direction::Outgoing) {
                        let mut stack = vec![neighbor];
                        while let Some(idx) = stack.pop() {
                            if dirty_set.insert(idx) {
                                for next in graph.neighbors_directed(idx, petgraph::Direction::Outgoing) {
                                    stack.push(next);
                                }
                            }
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
                        context.set_probe(input.id.0.clone(), hash, HashMap::new());
                        eprintln!("  {} {label}",
                            output::style::styled(output::style::status::PROBED, output::style::label::PROBED));
                    } else {
                        last_exit_code = 3;
                    }
                }

                _ => {
                    // Source nodes are probed eagerly before the DAG loop (for input_hash).
                    if matches!(&input.node, ResolvedNativeNode::Source { .. }) {
                        continue;
                    }
                    let result = probe::probe_input(&input.node);
                    if result.success {
                        // Inject probe variables for downstream commands
                        if !result.variables.is_empty() {
                            all_variables.extend(result.variables.clone());
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
    }

    // Report undeclared dependencies and poison cache if found
    let undeclared_bins: Vec<String> = undeclared_binaries.into_iter().collect();
    if !undeclared_bins.is_empty() {
        renderer.on_undeclared_deps(&undeclared_bins, &[]);
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
fn execute_command_with_retry(
    name: &str,
    run: &[String],
    env: &HashMap<String, String>,
    sandbox_env: &crate::ir::EnvSandboxResolved,
    workdir: Option<&str>,
    retry: Option<&crate::ir::RetryResolved>,
    _renderer: &mut dyn output::OutputRenderer,
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

        match tracer::execute_traced(run, env, sandbox_env, workdir) {
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
    renderer: &mut dyn output::OutputRenderer,
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
                let mut cmd_env = all_variables.clone();
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
    matches!(node, ResolvedNativeNode::Env {
        on_missing: crate::ir::types::OnMissingResolved::Skip, ..
    })
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
