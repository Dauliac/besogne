pub mod cache;
pub mod cli;
pub mod config;
mod verify;

use crate::ir::{BesogneIR, ResolvedNode, ResolvedNativeNode};
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

    // --status: show DAG tree + cached command replays, then exit
    if args.status {
        output::render_status_tree(&ir, &context);
        eprintln!();
        let all_vars = flag_vars;
        replay_cached_commands(&ir, &context, &mut *renderer, &all_vars);
        let wall_ms = start.elapsed().as_millis() as u64;
        renderer.on_summary(0, wall_ms);
        return ExitCode::SUCCESS;
    }

    renderer.on_start(&ir);

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

    // 1. Show build-sealed inputs — use Cached status on subsequent runs
    let build_status = if warmup_cached {
        output::ProbeStatus::Cached
    } else {
        output::ProbeStatus::Sealed
    };
    if !build_nodes.is_empty() {
        renderer.on_phase_start("build", build_nodes.len());
        for input in &build_nodes {
            let result = if let Some(snapshot) = &input.sealed {
                probe::ProbeResult {
                    success: true,
                    hash: snapshot.hash.clone(),
                    variables: HashMap::new(),
                    error: None,
                }
            } else {
                probe::ProbeResult {
                    success: true,
                    hash: String::new(),
                    variables: HashMap::new(),
                    error: None,
                }
            };
            renderer.on_probe_result(input, &result, build_status);
        }
        renderer.on_phase_end("build");
    }

    if warmup_cached {
        renderer.on_phase_start("seal", pre_nodes.len());
        let mut all_vars = flag_vars;
        let mut hash_parts = Vec::new();
        for input in &pre_nodes {
            if let Some(cached) = context.get_probe(&input.id.0) {
                let result = probe::ProbeResult {
                    success: true,
                    hash: cached.hash.clone(),
                    variables: cached.variables.clone(),
                    error: None,
                };
                renderer.on_probe_result(input, &result, output::ProbeStatus::Cached);
                all_vars.extend(cached.variables.clone());
                hash_parts.push(cached.hash.clone());
            }
        }
        renderer.on_phase_end("seal");

        hash_parts.sort();
        let input_hash = blake3::hash(hash_parts.join(":").as_bytes())
            .to_hex()
            .to_string();

        if !has_side_effects(&ir) && context.can_skip(&input_hash) {
            replay_cached_commands(&ir, &context, &mut *renderer, &all_vars);
            let wall_ms = start.elapsed().as_millis() as u64;
            renderer.on_summary(0, wall_ms);
            return ExitCode::SUCCESS;
        }

        return execute_dag(&ir, all_vars, input_hash, &mut *renderer, &mut context, start, args.force, args.debug);
    }

    // Full warmup: probe all seals in parallel
    renderer.on_phase_start("seal", pre_nodes.len());

    let all_variables = Mutex::new(flag_vars);
    let pre_hashes = Mutex::new(Vec::<String>::new());
    let pre_failed = Mutex::new(false);
    let probe_results = Mutex::new(Vec::new());

    crossbeam::scope(|s| {
        let handles: Vec<_> = pre_nodes
            .iter()
            .map(|input| {
                let all_vars = &all_variables;
                let hashes = &pre_hashes;
                let failed = &pre_failed;
                let results = &probe_results;

                s.spawn(move |_| {
                    let result = probe::probe_input(&input.node);
                    results.lock().unwrap().push(((*input).clone(), result.clone()));
                    if result.success {
                        all_vars.lock().unwrap().extend(result.variables.clone());
                        hashes.lock().unwrap().push(result.hash.clone());
                    } else {
                        *failed.lock().unwrap() = true;
                    }
                })
            })
            .collect();
        for h in handles { h.join().unwrap(); }
    })
    .unwrap();

    let mut results = probe_results.into_inner().unwrap();
    results.sort_by(|a, b| a.0.id.0.cmp(&b.0.id.0));
    for (input, result) in &results {
        renderer.on_probe_result(input, result, output::ProbeStatus::Fresh);
    }
    renderer.on_phase_end("seal");

    if *pre_failed.lock().unwrap() {
        let wall_ms = start.elapsed().as_millis() as u64;
        renderer.on_summary(2, wall_ms);
        return ExitCode::from(2);
    }

    let all_variables = all_variables.into_inner().unwrap();

    let mut hash_parts = pre_hashes.into_inner().unwrap();
    hash_parts.sort();
    let input_hash = blake3::hash(hash_parts.join(":").as_bytes())
        .to_hex()
        .to_string();

    if !has_side_effects(&ir) && context.can_skip(&input_hash) {
        replay_cached_commands(&ir, &context, &mut *renderer, &all_variables);
        let wall_ms = start.elapsed().as_millis() as u64;
        renderer.on_summary(0, wall_ms);
        return ExitCode::SUCCESS;
    }

    // Update cache with fresh probe results
    for (input, result) in &results {
        if result.success {
            context.set_probe(input.id.0.clone(), result.hash.clone(), result.variables.clone());
        }
    }

    execute_dag(&ir, all_variables, input_hash, &mut *renderer, &mut context, start, args.force, args.debug)
}

/// Execute the exec-phase DAG
fn execute_dag(
    ir: &BesogneIR,
    all_variables: HashMap<String, String>,
    input_hash: String,
    renderer: &mut dyn output::OutputRenderer,
    context: &mut ContextCache,
    start: Instant,
    force: bool,
    debug: bool,
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

    let mut undeclared_binaries: std::collections::HashSet<String> = std::collections::HashSet::new();

    // Idempotency verification: on first run, re-run each command (except side_effects)
    // to check whether outputs are deterministic.
    let first_run = !context.verified;
    let mut non_idempotent: Vec<String> = Vec::new();

    let input_by_id: HashMap<_, _> = ir
        .nodes
        .iter()
        .filter(|i| i.phase == Phase::Exec)
        .map(|i| (i.id.clone(), i))
        .collect();

    for tier in &tiers {
        for &node_idx in tier {
            let content_id = &graph[node_idx];
            let input = match input_by_id.get(content_id) {
                Some(i) => i,
                None => continue,
            };

            match &input.node {
                ResolvedNativeNode::Command {
                    name, run, env, side_effects, workdir, force_args, debug_args, ..
                } => {
                    if last_exit_code != 0 && !side_effects {
                        continue;
                    }

                    let mut cmd_env = all_variables.clone();
                    cmd_env.extend(env.clone());

                    // Append force_args/debug_args when --force/--debug is active
                    let mut effective_run = run.clone();
                    if force && !force_args.is_empty() {
                        effective_run.extend(force_args.clone());
                    }
                    if debug && !debug_args.is_empty() {
                        effective_run.extend(debug_args.clone());
                    }

                    let ctx = output::CommandContext {
                        binary_paths: &binary_paths,
                        binary_versions: &binary_versions,
                        env_vars: &cmd_env,
                        secret_vars: &secret_vars,
                    };
                    renderer.on_command_start(name, &effective_run, &ctx);

                    let result = match tracer::execute_traced(&effective_run, &cmd_env, &ir.sandbox.env, workdir.as_deref()) {
                        Ok(r) => r,
                        Err(e) => {
                            eprintln!("{}", crate::output::style::error_diag(&e.to_string()));
                            last_exit_code = 126;
                            continue;
                        }
                    };

                    let stdout = String::from_utf8_lossy(&result.stdout).to_string();
                    let cmd_stderr = String::from_utf8_lossy(&result.stderr).to_string();
                    renderer.on_command_output(name, &stdout, &cmd_stderr);
                    renderer.on_command_end(name, &result);

                    // Idempotency verification: re-run if first run + not side_effects + succeeded
                    if first_run && !side_effects && result.exit_code == 0 {
                        eprintln!("    {}", output::style::styled(
                            output::style::verify::VERIFYING,
                            output::style::message::VERIFY_RUN2,
                        ));
                        let mismatches = verify::verify_command(
                            &effective_run, &cmd_env, &ir.sandbox.env, workdir.as_deref(), &result,
                        );
                        eprintln!("{}", verify::format_verify_human(name, mismatches.as_deref()));
                        if let Some(reasons) = mismatches {
                            non_idempotent.push(name.clone());
                            let _ = reasons; // reasons already printed
                        }
                    }

                    // Detect undeclared binaries from process tree
                    let run_basenames: std::collections::HashSet<&str> = run.iter()
                        .filter_map(|a| a.rsplit('/').next())
                        .collect();
                    for (proc_idx, proc) in result.process_tree.iter().enumerate() {
                        if proc.comm.is_empty() { continue; }
                        if proc_idx == 0 { continue; } // skip root (the command itself)
                        let comm = &proc.comm;
                        if declared_binaries.contains(comm) { continue; }
                        if run_basenames.contains(comm.as_str()) { continue; }
                        // Skip shells, besogne itself, and hex-hash cache binary names
                        if ["bash", "sh", "dash", "zsh", "fish", "csh", "tcsh"].contains(&comm.as_str()) { continue; }
                        if comm.chars().all(|c| c.is_ascii_hexdigit()) { continue; }
                        if proc.cmdline.contains("besogne") { continue; }
                        undeclared_binaries.insert(comm.clone());
                    }

                    // Cache command output for replay on future skips.
                    // Skip cache writes when --debug is active to avoid poisoning
                    // cache with debug output (which changes stdout/stderr).
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
                            },
                        );
                    }

                    if result.exit_code != 0 {
                        last_exit_code = result.exit_code;
                    }
                }

                ResolvedNativeNode::Service { name, tcp, http, .. } => {
                    let label = name.as_deref().unwrap_or("service");
                    let target = tcp.as_deref().or(http.as_deref()).unwrap_or("?");
                    eprintln!("  waiting for {label} ({target})...");

                    let result = probe::probe_input(&input.node);
                    if !result.success {
                        eprintln!(
                            "  {} {label}: {}",
                            output::style::styled(output::style::status::FAILED, "✗"),
                            result.error.as_deref().unwrap_or("unreachable")
                        );
                        last_exit_code = 1;
                    }
                }

                _ => {
                    let result = probe::probe_input(&input.node);
                    if !result.success {
                        last_exit_code = 2;
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
        context.verified = true;
        context.non_idempotent = non_idempotent.clone();
        if !non_idempotent.is_empty() {
            eprintln!("\n  {} {}",
                output::style::styled(output::style::verify::NOT_IDEMPOTENT, output::style::message::NOT_IDEMPOTENT),
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
    renderer.on_summary(last_exit_code, wall_ms);
    ExitCode::from(last_exit_code as u8)
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

fn compute_besogne_hash(ir: &BesogneIR) -> String {
    let content = serde_json::to_string(ir).unwrap_or_default();
    blake3::hash(content.as_bytes()).to_hex()[..16].to_string()
}
