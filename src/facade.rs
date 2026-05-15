//! High-level library API for besogne.
//!
//! The [`Besogne`] struct is the main entry point for programmatic use.
//! Construct one via [`Besogne::builder()`], then call high-level operations
//! like [`build`](Besogne::build), [`run`](Besogne::run), [`check`](Besogne::check),
//! or [`list`](Besogne::list).
//!
//! # Example
//!
//! ```no_run
//! use besogne::Besogne;
//! use std::path::Path;
//!
//! let besogne = Besogne::builder()
//!     .force(true)
//!     .build();
//!
//! // Compile a manifest into a sealed binary
//! let output = besogne.build(Path::new("besogne.toml"), None)?;
//! println!("Binary at: {}", output.store_path.display());
//!
//! // Build + run, get structured output
//! let result = besogne.run(Path::new("besogne.toml"))?;
//! println!("exit={} wall={}ms commands={}", result.exit_code, result.wall_ms, result.commands.len());
//! for cmd in &result.commands {
//!     println!("  {} exit={} wall={}ms", cmd.name, cmd.exit_code, cmd.wall_ms);
//! }
//! # Ok::<(), besogne::error::BesogneError>(())
//! ```

use crate::compile;
use crate::error::BesogneError;
use crate::event::{BesogneEvent, EventHandler};
use crate::ir::BesogneIR;
use crate::manifest;
use crate::runtime;
use crate::runtime::cli::{LogFormat, RuntimeConfig};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::{Arc, Mutex};

// ── Structured output types ──

/// Output of a successful build operation.
pub struct BuildOutput {
    /// Path to the binary in the content-addressed store.
    pub store_path: PathBuf,
    /// BLAKE3 content hash of the binary.
    pub content_hash: String,
    /// Whether the binary was served from cache (no recompilation).
    pub from_cache: bool,
}

/// Output of a run operation — structured per-command results.
#[derive(Debug, Clone, Default)]
pub struct RunOutput {
    /// Overall exit code (0 = success).
    pub exit_code: i32,
    /// Total wall-clock time in milliseconds.
    pub wall_ms: u64,
    /// Whether the entire run was skipped (all inputs cached).
    pub skipped: bool,
    /// Per-command execution summaries, in execution order.
    pub commands: Vec<CommandSummary>,
}

/// Summary of a single command's execution.
#[derive(Debug, Clone)]
pub struct CommandSummary {
    /// Command name (from manifest key).
    pub name: String,
    /// Exit code (0 = success).
    pub exit_code: i32,
    /// Wall-clock time in milliseconds.
    pub wall_ms: u64,
    /// User CPU time in milliseconds.
    pub user_ms: u64,
    /// System CPU time in milliseconds.
    pub sys_ms: u64,
    /// Peak RSS in kilobytes.
    pub max_rss_kb: u64,
    /// Whether the command was served from cache.
    pub cached: bool,
}

/// Output of a check (validate) operation.
pub struct CheckOutput {
    /// The IR produced from the manifest (useful for inspection).
    pub ir: BesogneIR,
}

/// Information about a discovered manifest.
pub struct ManifestInfo {
    /// Path to the manifest file.
    pub path: PathBuf,
    /// Task name (derived from filename).
    pub name: String,
    /// Description from the manifest.
    pub description: String,
    /// Total number of nodes.
    pub node_count: usize,
    /// Number of command nodes.
    pub command_count: usize,
    /// Number of component nodes.
    pub component_count: usize,
}

// ── Event collector (internal) ──

/// Collects RunOutput data from events during execution.
/// Wrapped in Arc<Mutex> so we can extract data after run_with_handler takes ownership.
struct OutputCollector {
    data: Arc<Mutex<RunOutput>>,
}

impl OutputCollector {
    fn new() -> (Self, Arc<Mutex<RunOutput>>) {
        let data = Arc::new(Mutex::new(RunOutput::default()));
        (Self { data: Arc::clone(&data) }, data)
    }
}

impl EventHandler for OutputCollector {
    fn on_event(&mut self, event: &BesogneEvent<'_>) {
        let mut out = self.data.lock().unwrap();
        match event {
            BesogneEvent::CommandEnd { name, exit_code, wall_ms, result, .. } => {
                out.commands.push(CommandSummary {
                    name: name.to_string(),
                    exit_code: *exit_code,
                    wall_ms: *wall_ms,
                    user_ms: result.user_ms,
                    sys_ms: result.sys_ms,
                    max_rss_kb: result.max_rss_kb,
                    cached: false,
                });
            }
            BesogneEvent::CommandCached { name, cached, .. } => {
                out.commands.push(CommandSummary {
                    name: name.to_string(),
                    exit_code: cached.exit_code,
                    wall_ms: cached.wall_ms,
                    user_ms: cached.user_ms,
                    sys_ms: cached.sys_ms,
                    max_rss_kb: cached.max_rss_kb,
                    cached: true,
                });
            }
            BesogneEvent::Skip { .. } => {
                out.skipped = true;
            }
            BesogneEvent::Summary { exit_code, wall_ms } => {
                out.exit_code = *exit_code;
                out.wall_ms = *wall_ms;
            }
            _ => {}
        }
    }
}

// ── Core types ──

/// Main entry point for programmatic use of besogne.
///
/// Encapsulates configuration and provides high-level operations.
/// Construct via [`Besogne::builder()`].
pub struct Besogne {
    force: bool,
    log_format: LogFormat,
    verbose: bool,
    debug: bool,
    flag_env: HashMap<String, String>,
}

impl Default for Besogne {
    fn default() -> Self {
        Self {
            force: false,
            log_format: LogFormat::Human,
            verbose: false,
            debug: false,
            flag_env: HashMap::new(),
        }
    }
}

/// Builder for [`Besogne`].
pub struct BesogneBuilder {
    inner: Besogne,
}

impl BesogneBuilder {
    /// Force rebuild and re-execution (bypass all caches).
    pub fn force(mut self, force: bool) -> Self {
        self.inner.force = force;
        self
    }

    /// Set the output format for runtime execution.
    pub fn log_format(mut self, format: LogFormat) -> Self {
        self.inner.log_format = format;
        self
    }

    /// Enable verbose output (show cached probes, env values, process trees).
    pub fn verbose(mut self, verbose: bool) -> Self {
        self.inner.verbose = verbose;
        self
    }

    /// Enable debug mode (append debug_args, skip cache writes).
    pub fn debug(mut self, debug: bool) -> Self {
        self.inner.debug = debug;
        self
    }

    /// Suppress all output (quiet mode).
    pub fn quiet(mut self) -> Self {
        self.inner.verbose = false;
        self
    }

    /// Set flag environment variables (equivalent to CLI --flag-name values).
    pub fn flag_env(mut self, env: HashMap<String, String>) -> Self {
        self.inner.flag_env = env;
        self
    }

    /// Set a single flag environment variable.
    pub fn set_flag(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.inner.flag_env.insert(key.into(), value.into());
        self
    }

    /// Build the [`Besogne`] instance.
    pub fn build(self) -> Besogne {
        self.inner
    }
}

impl Besogne {
    /// Create a new builder with default configuration.
    pub fn builder() -> BesogneBuilder {
        BesogneBuilder {
            inner: Besogne::default(),
        }
    }

    /// Create an instance with default configuration.
    pub fn new() -> Self {
        Self::default()
    }

    // ── High-level operations ──

    /// Compile a manifest into a sealed binary.
    ///
    /// If `output_path` is `None`, the binary is stored in the content-addressed
    /// store only (no copy to a local path).
    pub fn build(&self, manifest_path: &Path, output_path: Option<&Path>) -> Result<BuildOutput, BesogneError> {
        if let Some(out) = output_path {
            let store_path = if self.verbose {
                compile::compile(manifest_path, out, self.force)?
            } else {
                let store = compile::compile_quiet(manifest_path, self.force)?;
                std::fs::copy(&store, out)
                    .map_err(|e| BesogneError::Compile(format!("cannot copy to {}: {e}", out.display())))?;
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    let _ = std::fs::set_permissions(out, std::fs::Permissions::from_mode(0o755));
                }
                store
            };
            Ok(build_output_from_store_path(store_path))
        } else {
            let store_path = compile::compile_quiet(manifest_path, self.force)?;
            Ok(build_output_from_store_path(store_path))
        }
    }

    /// Build and execute a manifest. Returns structured output with per-command metrics.
    pub fn run(&self, manifest_path: &Path) -> Result<RunOutput, BesogneError> {
        let ir = self.compile_and_extract(manifest_path)?;
        Ok(self.execute_ir(ir))
    }

    /// Build and execute, returning only the exit code (lower overhead, no collection).
    pub fn run_exit_code(&self, manifest_path: &Path) -> Result<ExitCode, BesogneError> {
        let ir = self.compile_and_extract(manifest_path)?;
        Ok(runtime::run_with_config(ir, &self.runtime_config()))
    }

    /// Validate a manifest without building. Returns the IR for inspection.
    pub fn check(&self, manifest_path: &Path) -> Result<CheckOutput, BesogneError> {
        let ir = compile::check_to_ir(manifest_path)?;
        compile::check(manifest_path)?;
        Ok(CheckOutput { ir })
    }

    /// Discover and list all manifests in the current directory.
    pub fn list(&self) -> Result<Vec<ManifestInfo>, BesogneError> {
        let discovered = manifest::discover_manifests();
        let mut infos = Vec::new();
        for path in discovered {
            let name = manifest::manifest_task_name(&path);
            match manifest::load_manifest(&path) {
                Ok(m) => {
                    let component_count = m.nodes.values()
                        .filter(|n| matches!(n, manifest::Node::Component(_)))
                        .count();
                    let command_count = m.nodes.values()
                        .filter(|n| matches!(n, manifest::Node::Command(_)))
                        .count();
                    infos.push(ManifestInfo {
                        path,
                        name,
                        description: m.description.clone(),
                        node_count: m.nodes.len(),
                        command_count,
                        component_count,
                    });
                }
                Err(_) => {
                    infos.push(ManifestInfo {
                        path,
                        name,
                        description: String::new(),
                        node_count: 0,
                        command_count: 0,
                        component_count: 0,
                    });
                }
            }
        }
        Ok(infos)
    }

    // ── Mid-level operations ──

    /// Parse and validate a manifest file.
    pub fn load_manifest(&self, path: &Path) -> Result<manifest::Manifest, BesogneError> {
        manifest::load_manifest(path)
    }

    /// Compile a manifest to IR without emitting a binary.
    /// Useful for inspection, IDE integration, or custom processing.
    pub fn compile_to_ir(&self, manifest_path: &Path) -> Result<BesogneIR, BesogneError> {
        compile::check_to_ir(manifest_path)
    }

    /// Execute a pre-compiled IR directly, returning structured output.
    pub fn execute_ir(&self, ir: BesogneIR) -> RunOutput {
        let (collector, data) = OutputCollector::new();
        runtime::run_with_handler(ir, &self.runtime_config(), Box::new(collector));
        Arc::try_unwrap(data)
            .expect("collector should be dropped after run completes")
            .into_inner()
            .unwrap()
    }

    /// Execute IR returning only the exit code (lower overhead).
    pub fn execute_ir_exit_code(&self, ir: BesogneIR) -> ExitCode {
        runtime::run_with_config(ir, &self.runtime_config())
    }

    // ── Event handler variants ──

    /// Build and execute with a custom event handler.
    ///
    /// The handler receives structured [`BesogneEvent`]s
    /// alongside the normal terminal output.
    pub fn run_with_handler(
        &self,
        manifest_path: &Path,
        handler: Box<dyn EventHandler>,
    ) -> Result<ExitCode, BesogneError> {
        let ir = self.compile_and_extract(manifest_path)?;
        Ok(runtime::run_with_handler(ir, &self.runtime_config(), handler))
    }

    /// Execute IR with a custom event handler.
    pub fn execute_ir_with_handler(
        &self,
        ir: BesogneIR,
        handler: Box<dyn EventHandler>,
    ) -> ExitCode {
        runtime::run_with_handler(ir, &self.runtime_config(), handler)
    }

    /// Build a [`RuntimeConfig`] from this instance's settings.
    pub fn runtime_config(&self) -> RuntimeConfig {
        RuntimeConfig {
            log_format: self.log_format.clone(),
            force: self.force,
            debug: self.debug,
            verbose: self.verbose,
            flag_env: self.flag_env.clone(),
            ..RuntimeConfig::default()
        }
    }

    // ── Internal helpers ──

    fn compile_and_extract(&self, manifest_path: &Path) -> Result<BesogneIR, BesogneError> {
        let store_path = compile::compile_quiet(manifest_path, self.force)?;
        compile::embed::extract_ir_from_binary(&store_path)
            .ok_or_else(|| BesogneError::Embed("cannot extract IR from compiled binary".into()))
    }
}

fn build_output_from_store_path(store_path: PathBuf) -> BuildOutput {
    let hash = store_path.parent()
        .and_then(|p| p.file_name())
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_string();
    BuildOutput {
        store_path,
        content_hash: hash,
        from_cache: false,
    }
}
