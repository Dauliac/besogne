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
#[command(name = "besogne", version, about = "Declarative contracts for shell commands — preconditions, sandboxing, memoization")]
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

    /// Validate manifest(s) without building
    Check {
        /// Path to manifest file(s). If omitted, auto-discovers.
        #[arg(num_args = 0..)]
        input: Vec<PathBuf>,
    },

    /// Build and run a manifest in one shot (build + pre + exec).
    /// All flags after `run` are forwarded to the produced besogne
    /// (e.g., `besogne run -l json --verbose` passes `-l json --verbose` to the binary).
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
fn resolve_inputs(explicit: &[PathBuf]) -> Result<Vec<PathBuf>, String> {
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

/// Hash the besogne compiler binary itself — used as part of the run cache key.
/// If besogne is updated, all cached produced binaries are invalidated.
fn self_hash() -> String {
    std::env::current_exe()
        .ok()
        .and_then(|p| std::fs::read(&p).ok())
        .map(|bytes| blake3::hash(&bytes).to_hex()[..16].to_string())
        .unwrap_or_else(|| "unknown".to_string())
}

fn main() -> ExitCode {
    // Check if we're a sealed besogne binary (has IR embedded)
    if let Some(ir_data) = compile::embed::extract_ir_from_self() {
        return runtime::run(ir_data);
    }

    // Otherwise, we're the builder CLI
    let cli = Cli::parse();

    match cli.command {
        Some(Commands::Build { input, output }) => {
            let inputs = match resolve_inputs(&input) {
                Ok(i) => i,
                Err(e) => { eprintln!("error: {e}"); return ExitCode::from(2); }
            };

            if inputs.len() > 1 && output.is_some() {
                eprintln!("error: --output cannot be used with multiple inputs");
                return ExitCode::from(2);
            }

            let mut failed = false;
            for manifest_path in &inputs {
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
            let manifest_path = match resolve_single_input_quiet(&input) {
                Ok(p) => p,
                Err(e) => { eprintln!("error: {e}"); return ExitCode::from(2); }
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
            let compiler_hash = self_hash();
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

            // exec replaces this process — the besogne binary takes over
            let err = exec_binary(&bin_path, &args);
            eprintln!("error: cannot exec {}: {err}", bin_path.display());
            ExitCode::from(126)
        }

        Some(Commands::Check { input }) => {
            let inputs = match resolve_inputs(&input) {
                Ok(i) => i,
                Err(e) => { eprintln!("error: {e}"); return ExitCode::from(2); }
            };

            let mut failed = false;
            for manifest_path in &inputs {
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
