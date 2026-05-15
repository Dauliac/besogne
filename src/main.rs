use besogne::adopt;
use besogne::compile;
use besogne::error;
use besogne::manifest;
use besogne::output;
use besogne::runtime;

use clap::{Parser, Subcommand};
use std::path::PathBuf;
use std::process::ExitCode;

#[derive(Parser)]
#[command(name = "besogne", version, about = "Declarative contracts for shell commands — seals, sandboxing, memoization")]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Build besogne(s): seal manifest(s) into self-contained binaries
    Build {
        /// Path to manifest file(s). If omitted, auto-discovers in current dir / git root.
        #[arg(short, long, num_args = 1..)]
        input: Vec<PathBuf>,

        /// Output binary path (only valid with a single input)
        #[arg(short, long)]
        output: Option<PathBuf>,

        /// Force rebuild — ignore content-addressed store cache
        #[arg(short, long)]
        force: bool,
    },

    /// Adopt scripts from package.json (or other task runners) into a besogne manifest.
    /// Parses scripts, detects binaries and side effects, generates besogne.toml,
    /// backs up the original file, and rewrites scripts to use `besogne run`.
    Adopt {
        /// Path to source file (e.g., package.json). Auto-detects format.
        #[arg(short, long)]
        source: PathBuf,

        /// Output manifest path (default: besogne.toml in same directory)
        #[arg(short, long)]
        output: Option<PathBuf>,

        /// Show what would be generated without writing files
        #[arg(long)]
        dry_run: bool,
    },

    /// Validate manifest(s) without building
    Check {
        /// Path to manifest file(s). If omitted, auto-discovers.
        #[arg(num_args = 0..)]
        input: Vec<PathBuf>,
    },

    /// List discovered manifests and their descriptions
    List {
        /// Show verbose details (components, node counts)
        #[arg(short, long)]
        verbose: bool,
    },

    /// Build and run in one shot. Use `besogne run -- --help` to see all flags.
    /// All arguments are forwarded to the produced binary (e.g., `besogne run -- -l json`).
    Run {
        /// Path to manifest file. If omitted, auto-discovers (must resolve to exactly one).
        #[arg(short, long)]
        input: Option<PathBuf>,

        /// Force rebuild + re-execute (bypass all caches)
        #[arg(short, long)]
        force: bool,

        /// All remaining arguments forwarded to the besogne binary
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
}

/// Handle `besogne run --help`: parse manifest (no compile), show merged grouped help.
fn handle_run_help(raw_args: &[String]) -> ExitCode {
    let mut input_path: Option<PathBuf> = None;
    let mut i = 2;
    while i < raw_args.len() {
        if (raw_args[i] == "-i" || raw_args[i] == "--input") && i + 1 < raw_args.len() {
            input_path = Some(PathBuf::from(&raw_args[i + 1]));
            i += 2;
        } else {
            i += 1;
        }
    }

    let remaining_args: Vec<String> = raw_args.iter().skip(2)
        .filter(|a| *a != "-i" && *a != "--input" && *a != "--help" && *a != "-h")
        .cloned().collect();
    let manifest_path = match manifest::resolve_single_manifest(&input_path, &remaining_args) {
        Ok((p, _)) => p,
        Err(e) => {
            eprintln!("besogne run — build + run in one shot\n");
            eprintln!("Usage: besogne run [<task>] [-i <manifest>] [-- FLAGS]\n");
            eprintln!("Run options:");
            eprintln!("  -i, --input <PATH>  Manifest file (auto-discovers if omitted)\n");
            eprintln!("Cannot show full help: {e}");
            return ExitCode::from(2);
        }
    };

    let ir = match compile::check_to_ir(&manifest_path) {
        Ok(ir) => ir,
        Err(e) => {
            eprintln!("{}", output::style::error_diag(&e.to_string()));
            return ExitCode::from(2);
        }
    };

    eprintln!("besogne run — build + run in one shot");
    eprintln!("manifest: {}\n", manifest_path.display());
    eprintln!("Run options:");
    eprintln!("  -i, --input <PATH>  Manifest file (auto-discovers if omitted)\n");

    let mut cmd = runtime::cli::build_runtime_cli(&ir);
    let mut buf = Vec::new();
    cmd.write_long_help(&mut buf).ok();
    eprint!("{}", String::from_utf8_lossy(&buf));

    ExitCode::SUCCESS
}

fn main() -> ExitCode {
    // Check if we're a sealed besogne binary (has IR embedded)
    if let Some(ir_data) = compile::embed::extract_ir_from_self() {
        return runtime::run(ir_data);
    }

    // Intercept `besogne run --help` BEFORE clap (clap would consume it)
    let raw_args: Vec<String> = std::env::args().collect();
    if raw_args.len() >= 3 && raw_args[1] == "run"
        && raw_args.iter().skip(2).any(|a| a == "--help" || a == "-h")
    {
        return handle_run_help(&raw_args);
    }

    let cli = Cli::parse();

    match cli.command {
        Some(Commands::Build { input, output, force }) => {
            let manifests = match manifest::resolve_manifests(&input) {
                Ok(i) => i,
                Err(e) => { eprintln!("{}", output::style::error_diag(&e.to_string())); return ExitCode::from(2); }
            };

            if manifests.len() > 1 && output.is_some() {
                eprintln!("{}", output::style::error_diag("--output cannot be used with multiple manifests"));
                return ExitCode::from(2);
            }

            let cwd = std::env::current_dir().unwrap_or_default();

            let tasks: Vec<(PathBuf, PathBuf, String)> = manifests.iter().map(|manifest_path| {
                let name = manifest::manifest_task_name(manifest_path);
                let out = output.clone().unwrap_or_else(|| {
                    let dir = cwd.join(".besogne");
                    let _ = std::fs::create_dir_all(&dir);
                    dir.join(&name)
                });
                (manifest_path.clone(), out, name)
            }).collect();

            if tasks.len() == 1 {
                let (manifest_path, out, name) = &tasks[0];
                match compile::compile(manifest_path, out, force) {
                    Ok(store_path) => {
                        if output.is_none() {
                            compile::create_besogne_symlink(&cwd, name, &store_path);
                            eprintln!(
                                "besogne: built {} → .besogne/{} (store: {})",
                                manifest_path.display(), name,
                                compile::store_path_short(&store_path)
                            );
                        } else {
                            eprintln!("besogne: built {} → {}", manifest_path.display(), out.display());
                        }
                        ExitCode::SUCCESS
                    }
                    Err(e) => {
                        eprintln!("{}", output::style::error_diag(&format!("{}: {e}", manifest_path.display())));
                        ExitCode::from(2)
                    }
                }
            } else {
                let build_start = std::time::Instant::now();
                eprintln!("besogne: building {} manifests in parallel...", tasks.len());

                let results: std::sync::Mutex<Vec<(String, PathBuf, Result<PathBuf, error::BesogneError>)>> =
                    std::sync::Mutex::new(Vec::new());

                crossbeam::scope(|s| {
                    for (manifest_path, _out, name) in &tasks {
                        let results = &results;
                        s.spawn(move |_| {
                            let result = compile::compile_quiet(manifest_path, force);
                            results.lock().unwrap().push((name.clone(), manifest_path.clone(), result));
                        });
                    }
                }).unwrap();

                let mut failed = false;
                let mut results = results.into_inner().unwrap();
                results.sort_by(|a, b| a.0.cmp(&b.0));

                for (name, manifest_path, result) in &results {
                    match result {
                        Ok(store_path) => {
                            if output.is_none() {
                                compile::create_besogne_symlink(&cwd, name, store_path);
                            }
                            eprintln!("  {} {name} {}",
                                output::style::styled(output::style::status::FRESH, "✓"),
                                output::style::dim(&manifest_path.display().to_string()));
                        }
                        Err(e) => {
                            eprintln!("  {} {name}: {e}",
                                output::style::styled(output::style::status::FAILED, "✗"));
                            failed = true;
                        }
                    }
                }

                let total_ms = build_start.elapsed().as_millis();
                eprintln!("besogne: built {} manifests ({})",
                    results.len(), output::style::format_duration(total_ms));

                if failed { ExitCode::from(2) } else { ExitCode::SUCCESS }
            }
        }

        Some(Commands::Run { input, force, args }) => {
            let wants_help = args.iter().any(|a| a == "--help" || a == "-h");

            let (manifest_path, forwarded_args) = match manifest::resolve_single_manifest(&input, &args) {
                Ok((p, remaining)) => (p, remaining.to_vec()),
                Err(e) => {
                    if wants_help {
                        eprintln!("besogne run: build + run a manifest in one shot");
                        eprintln!("  All flags after 'run' are forwarded to the produced binary.\n");
                        eprintln!("{}", output::style::error_diag(&e.to_string()));
                        eprintln!("  Cannot show full help without a manifest to build.\n");
                        eprintln!("Usage: besogne run [<task>] [-i <manifest>] [-- <flags>]");
                    } else {
                        eprintln!("{}", output::style::error_diag(&e.to_string()));
                    }
                    return ExitCode::from(2);
                }
            };

            let force_rebuild = force;
            let mut forwarded_args = forwarded_args;
            if force { forwarded_args.push("--force".to_string()); }

            let json_mode = forwarded_args.windows(2).any(|w| {
                (w[0] == "-l" || w[0] == "--log-format") && w[1] == "json"
            });

            let run_dir = runtime::cache::cache_base_dir().join("run");
            let _ = std::fs::create_dir_all(&run_dir);

            let manifest_content = match std::fs::read(&manifest_path) {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("{}", output::style::error_diag(&format!("cannot read {}: {e}", manifest_path.display())));
                    return ExitCode::from(2);
                }
            };
            let compiler_hash = runtime::cache::compiler_self_hash();
            let mut hasher = blake3::Hasher::new();
            hasher.update(&manifest_content);
            hasher.update(compiler_hash.as_bytes());
            let cache_key = hasher.finalize().to_hex()[..16].to_string();
            let bin_path = run_dir.join(&cache_key);

            let needs_build = force_rebuild || !bin_path.exists();

            if needs_build {
                match compile::compile_quiet(&manifest_path, force_rebuild) {
                    Ok(store_path) => {
                        if let Err(e) = std::fs::copy(&store_path, &bin_path) {
                            eprintln!("{}", output::style::error_diag(&format!("cannot copy to run cache: {e}")));
                            return ExitCode::from(2);
                        }
                        #[cfg(unix)]
                        {
                            use std::os::unix::fs::PermissionsExt;
                            let _ = std::fs::set_permissions(&bin_path, std::fs::Permissions::from_mode(0o755));
                        }
                    }
                    Err(e) => {
                        eprintln!("{}", output::style::error_diag(&e.to_string()));
                        return ExitCode::from(2);
                    }
                }
                if json_mode {
                    eprintln!(
                        "{}",
                        serde_json::json!({
                            "event": "build",
                            "status": if force_rebuild { "forced" } else { "built" },
                            "manifest": manifest_path.display().to_string(),
                            "binary": bin_path.display().to_string(),
                            "cache_key": cache_key,
                        })
                    );
                }
            } else if json_mode {
                eprintln!(
                    "{}",
                    serde_json::json!({
                        "event": "build",
                        "status": "cached",
                        "binary": bin_path.display().to_string(),
                        "cache_key": cache_key,
                    })
                );
            }

            if wants_help {
                eprintln!("besogne run — build + run in one shot");
                eprintln!("manifest: {}", manifest_path.display());
                eprintln!("binary:   {}\n", bin_path.display());
            }

            let err = runtime::exec_binary(&bin_path, &forwarded_args);
            eprintln!("{}", output::style::error_diag(&format!("cannot exec {}: {err}", bin_path.display())));
            ExitCode::from(126)
        }

        Some(Commands::Adopt { source, output, dry_run }) => {
            let source_type = match source.extension().and_then(|e| e.to_str()) {
                Some("json") => {
                    let name = source.file_name().and_then(|f| f.to_str()).unwrap_or("");
                    if name == "package.json" {
                        adopt::AdoptSource::PackageJson
                    } else {
                        eprintln!("{}", output::style::error_diag("unsupported source file (currently only package.json is supported)"));
                        return ExitCode::from(2);
                    }
                }
                _ => {
                    eprintln!("{}", output::style::error_diag("unsupported source format (currently only package.json is supported)"));
                    return ExitCode::from(2);
                }
            };

            let output_path = output.unwrap_or_else(|| {
                source.parent().unwrap_or(std::path::Path::new(".")).join("besogne.toml")
            });

            match adopt::adopt(&source, &source_type, &output_path, dry_run) {
                Ok(result) => {
                    eprintln!("besogne adopt: {} scripts adopted", result.scripts.len());
                    if !dry_run {
                        eprintln!("  manifest: {}", result.manifest_path.display());
                        eprintln!("  backup:   {}", result.backup_path.display());
                        eprintln!("\nnext steps:");
                        eprintln!("  1. review {}", result.manifest_path.display());
                        eprintln!("  2. besogne run --verify  (check idempotency)");
                        eprintln!("  3. remove side_effects = true from idempotent commands");
                    }
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("{}", output::style::error_diag(&e.to_string()));
                    ExitCode::from(2)
                }
            }
        }

        Some(Commands::Check { input }) => {
            let manifests = match manifest::resolve_manifests(&input) {
                Ok(i) => i,
                Err(e) => { eprintln!("{}", output::style::error_diag(&e.to_string())); return ExitCode::from(2); }
            };

            let mut failed = false;
            for manifest_path in &manifests {
                match compile::check(manifest_path) {
                    Ok(()) => {
                        eprintln!("besogne: {} is valid", manifest_path.display());
                    }
                    Err(e) => {
                        eprintln!("{}", output::style::error_diag(&format!("{}: {e}", manifest_path.display())));
                        failed = true;
                    }
                }
            }

            if failed { ExitCode::from(2) } else { ExitCode::SUCCESS }
        }

        Some(Commands::List { verbose }) => {
            let discovered = manifest::discover_manifests();
            if discovered.is_empty() {
                eprintln!("besogne: no manifests found");
                return ExitCode::from(1);
            }

            let cwd = std::env::current_dir().unwrap_or_default();
            for manifest_path in &discovered {
                let display_path = manifest_path
                    .strip_prefix(&cwd)
                    .unwrap_or(manifest_path)
                    .display();

                match manifest::load_manifest(manifest_path) {
                    Ok(m) => {
                        let name = manifest::manifest_task_name(manifest_path);

                        if verbose {
                            eprintln!("  {display_path}");
                            eprintln!("    {}", m.description);
                            let component_count = m.nodes.values()
                                .filter(|n| matches!(n, manifest::Node::Component(_)))
                                .count();
                            let command_count = m.nodes.values()
                                .filter(|n| matches!(n, manifest::Node::Command(_)))
                                .count();
                            let total = m.nodes.len();
                            eprintln!("    nodes: {total} ({command_count} commands, {component_count} components)");
                            eprintln!();
                        } else {
                            eprintln!("  {name:<16}{}", m.description);
                        }
                    }
                    Err(e) => {
                        eprintln!("  {display_path}  (error: {e})");
                    }
                }
            }

            let besogne_count = discovered.iter()
                .filter(|p| p.parent().and_then(|d| d.file_name())
                    .map(|n| n == "besogne").unwrap_or(false))
                .count();
            if besogne_count > 0 {
                eprintln!("\n  {besogne_count} tasks in besogne/");
            }

            ExitCode::SUCCESS
        }

        None => {
            eprintln!("besogne: no command specified. Use 'besogne build', 'besogne run', 'besogne list', or 'besogne check'.");
            eprintln!("         Or this binary has no embedded manifest.");
            ExitCode::from(1)
        }
    }
}
