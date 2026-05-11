pub mod cache;
pub mod cli;
pub mod config;
pub mod verify;

use crate::ir::{BesogneIR, ResolvedInput, ResolvedNativeInput};
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

    let mut renderer = output::renderer_for_format(&args.log_format);

    let start = Instant::now();
    renderer.on_start(&ir);

    let flag_vars = args.flag_env;

    // Compute besogne hash for memoization
    let besogne_hash = compute_besogne_hash(&ir);
    let mut context = ContextCache::load(&besogne_hash);

    // Partition inputs by phase
    let build_inputs: Vec<&ResolvedInput> = ir.inputs.iter().filter(|i| i.phase == Phase::Build).collect();
    let pre_inputs: Vec<&ResolvedInput> = ir.inputs.iter().filter(|i| i.phase == Phase::Pre).collect();

    // 1. Show build-sealed inputs (resolved at compile time)
    if !build_inputs.is_empty() {
        renderer.on_phase_start("build", build_inputs.len());
        for input in &build_inputs {
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
            renderer.on_probe_result(input, &result, output::ProbeStatus::Sealed);
        }
        renderer.on_phase_end("build");
    }

    // 2. Pre-phase — check preconditions
    let warmup_cached = !args.force && !args.verify && pre_inputs.iter().all(|input| {
        if context.get_probe(&input.id.0).is_none() {
            return false;
        }
        // For file inputs, verify the file hasn't changed since caching
        // by re-hashing (cheap for small files, catches content changes)
        match &input.input {
            ResolvedNativeInput::File { path, .. } => {
                let fresh = probe::probe_input(&input.input);
                if let Some(cached) = context.get_probe(&input.id.0) {
                    fresh.success && fresh.hash == cached.hash
                } else {
                    false
                }
            }
            _ => true,
        }
    });

    if warmup_cached {
        renderer.on_phase_start("pre", pre_inputs.len());
        let mut all_vars = flag_vars;
        let mut hash_parts = Vec::new();
        for input in &pre_inputs {
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
        renderer.on_phase_end("pre");

        hash_parts.sort();
        let input_hash = blake3::hash(hash_parts.join(":").as_bytes())
            .to_hex()
            .to_string();

        if !has_side_effects(&ir) && context.can_skip(&input_hash) {
            replay_cached_commands(&ir, &context, &mut *renderer);
            let wall_ms = start.elapsed().as_millis() as u64;
            renderer.on_skip("preconditions cached, postconditions valid");
            renderer.on_summary(0, wall_ms);
            return ExitCode::SUCCESS;
        }

        return execute_dag(&ir, all_vars, input_hash, &mut *renderer, &mut context, start);
    }

    // Full warmup: probe all preconditions in parallel
    renderer.on_phase_start("pre", pre_inputs.len());

    let all_variables = Mutex::new(flag_vars);
    let pre_hashes = Mutex::new(Vec::<String>::new());
    let pre_failed = Mutex::new(false);
    let probe_results = Mutex::new(Vec::new());

    crossbeam::scope(|s| {
        let handles: Vec<_> = pre_inputs
            .iter()
            .map(|input| {
                let all_vars = &all_variables;
                let hashes = &pre_hashes;
                let failed = &pre_failed;
                let results = &probe_results;

                s.spawn(move |_| {
                    let result = probe::probe_input(&input.input);
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
    renderer.on_phase_end("pre");

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
        replay_cached_commands(&ir, &context, &mut *renderer);
        let wall_ms = start.elapsed().as_millis() as u64;
        renderer.on_skip("preconditions unchanged, postconditions valid");
        renderer.on_summary(0, wall_ms);
        return ExitCode::SUCCESS;
    }

    // Update cache with fresh probe results
    for (input, result) in &results {
        if result.success {
            context.set_probe(input.id.0.clone(), result.hash.clone(), result.variables.clone());
        }
    }

    // Idempotency verification mode: run twice, compare
    if args.verify {
        let results = verify::verify_idempotency(&ir, &all_variables, &mut *renderer);
        let all_ok = results.iter().all(|r| r.idempotent || r.side_effects_declared);
        let wall_ms = start.elapsed().as_millis() as u64;
        renderer.on_summary(if all_ok { 0 } else { 3 }, wall_ms);
        return ExitCode::from(if all_ok { 0 } else { 3 });
    }

    execute_dag(&ir, all_variables, input_hash, &mut *renderer, &mut context, start)
}

/// Execute the exec-phase DAG
fn execute_dag(
    ir: &BesogneIR,
    all_variables: HashMap<String, String>,
    input_hash: String,
    renderer: &mut dyn output::OutputRenderer,
    context: &mut ContextCache,
    start: Instant,
) -> ExitCode {
    let (graph, _) = match dag::build_exec_dag(ir) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::from(2);
        }
    };

    let tiers = match dag::compute_tiers(&graph) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::from(2);
        }
    };

    let mut last_exit_code = 0;

    let input_by_id: HashMap<_, _> = ir
        .inputs
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

            match &input.input {
                ResolvedNativeInput::Command {
                    name, run, env, side_effects, output, ..
                } => {
                    if last_exit_code != 0 && !side_effects {
                        continue;
                    }

                    let mut cmd_env = all_variables.clone();
                    cmd_env.extend(env.clone());

                    renderer.on_command_start(name, run);

                    let result = match tracer::execute_traced(run, &cmd_env, &ir.sandbox.env) {
                        Ok(r) => r,
                        Err(e) => {
                            eprintln!("error: {e}");
                            last_exit_code = 126;
                            continue;
                        }
                    };

                    let stdout = String::from_utf8_lossy(&result.stdout).to_string();
                    let cmd_stderr = String::from_utf8_lossy(&result.stderr).to_string();
                    renderer.on_command_output(name, &stdout, &cmd_stderr);
                    renderer.on_command_end(name, &result);

                    // Cache command output for replay on future skips
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
                            ran_at: chrono::Utc::now().to_rfc3339(),
                        },
                    );

                    // Validate output assertions (opt-in)
                    if let Some(spec) = output {
                        if let Err(e) = verify::check_output(spec, &stdout, &cmd_stderr, result.exit_code) {
                            eprintln!("  \x1b[31mfail\x1b[0m {name} output: {e}");
                            last_exit_code = 3;
                            continue;
                        }
                    }

                    if result.exit_code != 0 {
                        last_exit_code = result.exit_code;
                    }
                }

                ResolvedNativeInput::Service { name, tcp, http, .. } => {
                    let label = name.as_deref().unwrap_or("service");
                    let target = tcp.as_deref().or(http.as_deref()).unwrap_or("?");
                    eprintln!("  waiting for {label} ({target})...");

                    let result = probe::probe_input(&input.input);
                    if !result.success {
                        eprintln!(
                            "  \x1b[31m✗\x1b[0m {label}: {}",
                            result.error.as_deref().unwrap_or("unreachable")
                        );
                        last_exit_code = 1;
                    }
                }

                _ => {
                    let result = probe::probe_input(&input.input);
                    if !result.success {
                        last_exit_code = 2;
                    }
                }
            }
        }
    }

    let wall_ms = start.elapsed().as_millis() as u64;
    context.set_last_run(input_hash, last_exit_code, wall_ms);
    let _ = context.save();
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

            let build_inputs: Vec<_> = ir.inputs.iter().filter(|i| i.phase == Phase::Build).collect();
            let pre_inputs: Vec<_> = ir.inputs.iter().filter(|i| i.phase == Phase::Pre).collect();
            let exec_inputs: Vec<_> = ir.inputs.iter().filter(|i| i.phase == Phase::Exec).collect();

            if !build_inputs.is_empty() {
                println!("Sealed (build phase) ({}):", build_inputs.len());
                for i in &build_inputs { println!("  {}", i.id); }
                println!();
            }
            if !pre_inputs.is_empty() {
                println!("Preconditions (pre phase) ({}):", pre_inputs.len());
                for i in &pre_inputs { println!("  {}", i.id); }
                println!();
            }
            if !exec_inputs.is_empty() {
                println!("Execution (exec phase) ({}):", exec_inputs.len());
                for i in &exec_inputs { println!("  {}", i.id); }
                println!();
            }

            let se_count = ir.inputs.iter().filter(|i| matches!(&i.input,
                ResolvedNativeInput::Command { side_effects: true, .. }
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
) {
    for input in &ir.inputs {
        if input.phase != Phase::Exec {
            continue;
        }
        if let ResolvedNativeInput::Command { name, run, .. } = &input.input {
            if let Some(cached) = context.get_command(name) {
                renderer.on_command_cached(name, run, cached);
            }
        }
    }
}

/// Returns true if any exec-phase command has side_effects = true
fn has_side_effects(ir: &BesogneIR) -> bool {
    ir.inputs.iter().any(|i| matches!(&i.input,
        ResolvedNativeInput::Command { side_effects: true, .. }
    ))
}

fn compute_besogne_hash(ir: &BesogneIR) -> String {
    let content = serde_json::to_string(ir).unwrap_or_default();
    blake3::hash(content.as_bytes()).to_hex()[..16].to_string()
}
