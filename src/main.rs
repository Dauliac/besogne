mod adopt;
mod compile;
mod ir;
mod manifest;
mod output;
mod probe;
mod runtime;
mod tracer;

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

    /// Build and run in one shot. Use `besogne run -- --help` to see all flags.
    /// All arguments are forwarded to the produced binary (e.g., `besogne run -- -l json`).
    Run {
        /// Path to manifest file. If omitted, auto-discovers (must resolve to exactly one).
        #[arg(short, long)]
        input: Option<PathBuf>,

        /// All remaining arguments forwarded to the besogne binary
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
}

/// Resolve inputs: use explicit paths or auto-discover.
fn resolve_manifests(explicit: &[PathBuf]) -> Result<Vec<PathBuf>, String> {
    if !explicit.is_empty() {
        return Ok(explicit.to_vec());
    }
    let discovered = manifest::discover_manifests();
    if discovered.is_empty() {
        return Err("no manifest found. Provide --input or create a besogne.{json,yaml,yml,toml} file.".into());
    }
    eprintln!(
        "besogne: discovered {}",
        discovered.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join(", ")
    );
    Ok(discovered)
}

/// Resolve a single input for `run`.
fn resolve_single_input(explicit: &Option<PathBuf>) -> Result<PathBuf, String> {
    if let Some(p) = explicit {
        return Ok(p.clone());
    }
    let discovered = manifest::discover_manifests();
    match discovered.len() {
        0 => Err("no manifest found. Provide --input or create a besogne.{json,yaml,yml,toml} file.".into()),
        1 => {
            eprintln!("besogne: discovered {}", discovered[0].display());
            Ok(discovered[0].clone())
        }
        n => Err(format!(
            "found {n} manifests — ambiguous for `run`. Use -i to pick one:\n  {}",
            discovered.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join("\n  ")
        )),
    }
}

/// Quiet variant — no discovery logs (for `run` mode where besogne binary handles output)
fn resolve_single_input_quiet(explicit: &Option<PathBuf>) -> Result<PathBuf, String> {
    if let Some(p) = explicit {
        return Ok(p.clone());
    }
    let discovered = manifest::discover_manifests();
    match discovered.len() {
        0 => Err("no manifest found. Provide --input or create a besogne.{json,yaml,yml,toml} file.".into()),
        1 => Ok(discovered[0].clone()),
        n => Err(format!(
            "found {n} manifests — ambiguous for `run`. Use -i to pick one:\n  {}",
            discovered.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join("\n  ")
        )),
    }
}

#[cfg(unix)]
fn exec_binary(path: &PathBuf, args: &[String]) -> std::io::Error {
    use std::os::unix::process::CommandExt;
    std::process::Command::new(path).args(args).exec()
}

#[cfg(not(unix))]
fn exec_binary(path: &PathBuf, args: &[String]) -> std::io::Error {
    match std::process::Command::new(path).args(args).status() {
        Ok(status) => std::process::exit(status.code().unwrap_or(1)),
        Err(e) => e,
    }
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

    let manifest_path = match resolve_single_input_quiet(&input_path) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("besogne run — build + run in one shot\n");
            eprintln!("Usage: besogne run [-i <manifest>] [FLAGS]\n");
            eprintln!("Run options:");
            eprintln!("  -i, --input <PATH>  Manifest file (auto-discovers if omitted)\n");
            eprintln!("Cannot show full help: {e}");
            return ExitCode::from(2);
        }
    };

    // Just PARSE — no compile needed for help
    let ir = match compile::check_to_ir(&manifest_path) {
        Ok(ir) => ir,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::from(2);
        }
    };

    // Print header
    eprintln!("besogne run — build + run in one shot");
    eprintln!("manifest: {}\n", manifest_path.display());
    eprintln!("Run options:");
    eprintln!("  -i, --input <PATH>  Manifest file (auto-discovers if omitted)\n");

    // Build the clap Command from IR and print its help (with grouped headings)
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

    // Otherwise, we're the builder CLI
    let cli = Cli::parse();

    match cli.command {
        Some(Commands::Build { input, output }) => {
            let manifests = match resolve_manifests(&input) {
                Ok(i) => i,
                Err(e) => { eprintln!("error: {e}"); return ExitCode::from(2); }
            };

            if manifests.len() > 1 && output.is_some() {
                eprintln!("error: --output cannot be used with multiple manifests");
                return ExitCode::from(2);
            }

            let mut failed = false;
            for manifest_path in &manifests {
                let out = output.clone().unwrap_or_else(|| {
                    // Derive output name from manifest: foo.besogne.json → foo
                    let stem = manifest_path.file_stem()
                        .and_then(|s| s.to_str())
                        .unwrap_or("besogne");
                    // Strip .besogne suffix if present
                    let name = stem.strip_suffix(".besogne").unwrap_or(stem);
                    manifest_path.parent().unwrap_or(std::path::Path::new(".")).join(name)
                });

                match compile::compile(manifest_path, &out) {
                    Ok(()) => {
                        eprintln!("besogne: built {} → {}", manifest_path.display(), out.display());
                    }
                    Err(e) => {
                        eprintln!("error: {}: {e}", manifest_path.display());
                        failed = true;
                    }
                }
            }

            if failed { ExitCode::from(2) } else { ExitCode::SUCCESS }
        }

        Some(Commands::Run { input, args }) => {
            // If --help is in args, we need to build first then forward --help
            // to show the produced binary's help (which includes all flags)
            let wants_help = args.iter().any(|a| a == "--help" || a == "-h");

            let manifest_path = match resolve_single_input_quiet(&input) {
                Ok(p) => p,
                Err(e) => {
                    if wants_help {
                        eprintln!("besogne run: build + run a manifest in one shot");
                        eprintln!("  All flags after 'run' are forwarded to the produced binary.\n");
                        eprintln!("error: {e}");
                        eprintln!("  Cannot show full help without a manifest to build.\n");
                        eprintln!("Usage: besogne run [-i <manifest>] [-- <flags>]");
                    } else {
                        eprintln!("error: {e}");
                    }
                    return ExitCode::from(2);
                }
            };

            // Detect -f/--force in forwarded args to force rebuild
            let force_rebuild = args.iter().any(|a| a == "-f" || a == "--force");

            // Detect JSON output mode from forwarded args
            let json_mode = args.windows(2).any(|w| {
                (w[0] == "-l" || w[0] == "--log-format") && w[1] == "json"
            });

            let cache_dir = std::env::var("XDG_CACHE_HOME").unwrap_or_else(|_| {
                let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
                format!("{home}/.cache")
            });
            let run_dir = PathBuf::from(&cache_dir).join("besogne").join("run");
            let _ = std::fs::create_dir_all(&run_dir);

            // Cache key: H(manifest_content + besogne_compiler_hash)
            // If besogne is updated, all cached binaries are invalidated.
            let manifest_content = match std::fs::read(&manifest_path) {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("error: cannot read {}: {e}", manifest_path.display());
                    return ExitCode::from(2);
                }
            };
            let compiler_hash = runtime::cache::compiler_self_hash();
            let mut hasher = blake3::Hasher::new();
            hasher.update(&manifest_content);
            hasher.update(compiler_hash.as_bytes());
            let cache_key = hasher.finalize().to_hex()[..16].to_string();
            let bin_path = run_dir.join(&cache_key);

            // Check if cached binary exists and is valid
            let needs_build = force_rebuild || !bin_path.exists();

            if needs_build {
                if let Err(e) = compile::compile_quiet(&manifest_path, &bin_path) {
                    eprintln!("error: {e}");
                    return ExitCode::from(2);
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

            // When --help: show context header before forwarding
            if wants_help {
                eprintln!("besogne run — build + run in one shot");
                eprintln!("manifest: {}", manifest_path.display());
                eprintln!("binary:   {}\n", bin_path.display());
            }

            // exec replaces this process — the besogne binary takes over
            let err = exec_binary(&bin_path, &args);
            eprintln!("error: cannot exec {}: {err}", bin_path.display());
            ExitCode::from(126)
        }

        Some(Commands::Adopt { source, output, dry_run }) => {
            let source_type = match source.extension().and_then(|e| e.to_str()) {
                Some("json") => {
                    // Check if it's package.json
                    let name = source.file_name().and_then(|f| f.to_str()).unwrap_or("");
                    if name == "package.json" {
                        adopt::AdoptSource::PackageJson
                    } else {
                        eprintln!("error: unsupported source file. Currently only package.json is supported.");
                        return ExitCode::from(2);
                    }
                }
                _ => {
                    eprintln!("error: unsupported source format. Currently only package.json is supported.");
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
                    eprintln!("error: {e}");
                    ExitCode::from(2)
                }
            }
        }

        Some(Commands::Check { input }) => {
            let manifests = match resolve_manifests(&input) {
                Ok(i) => i,
                Err(e) => { eprintln!("error: {e}"); return ExitCode::from(2); }
            };

            let mut failed = false;
            for manifest_path in &manifests {
                match compile::check(manifest_path) {
                    Ok(()) => {
                        eprintln!("besogne: {} is valid", manifest_path.display());
                    }
                    Err(e) => {
                        eprintln!("error: {}: {e}", manifest_path.display());
                        failed = true;
                    }
                }
            }

            if failed { ExitCode::from(2) } else { ExitCode::SUCCESS }
        }

        None => {
            eprintln!("besogne: no command specified. Use 'besogne build', 'besogne run', or 'besogne check'.");
            eprintln!("         Or this binary has no embedded manifest.");
            ExitCode::from(1)
        }
    }
}
