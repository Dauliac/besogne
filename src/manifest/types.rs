use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Top-level manifest structure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    pub name: String,
    pub description: String,

    /// Sandbox configuration (effect handler)
    #[serde(default)]
    pub sandbox: Option<Sandbox>,

    #[serde(default)]
    pub flags: Vec<Flag>,

    /// Component sources: namespace → source.
    /// "builtin" for embedded components, "./path" for local, "github:org/repo#ref" for remote.
    #[serde(default)]
    pub components: HashMap<String, String>,

    #[serde(default)]
    pub nodes: HashMap<String, Node>,
}

/// A user-defined flag for the produced besogne binary
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Flag {
    pub name: String,

    #[serde(default)]
    pub short: Option<char>,

    #[serde(default)]
    pub description: Option<String>,

    /// Long-form documentation for man pages and --help
    #[serde(default)]
    pub doc: Option<String>,

    #[serde(default = "FlagKind::default_kind")]
    pub kind: FlagKind,

    #[serde(default)]
    pub default: Option<serde_json::Value>,

    #[serde(default)]
    pub values: Option<Vec<String>>,

    #[serde(default)]
    pub required: Option<bool>,

    #[serde(default)]
    pub env: Option<String>,

    #[serde(default)]
    pub subcommand: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FlagKind {
    Bool,
    String,
    Positional,
}

impl FlagKind {
    fn default_kind() -> Self {
        FlagKind::Bool
    }
}

/// Sandbox configuration (effect handler)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Sandbox {
    Preset(SandboxPreset),
    Custom(SandboxConfig),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SandboxPreset {
    None,
    Strict,
    Container,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SandboxConfig {
    #[serde(default)]
    pub preset: Option<SandboxPreset>,

    #[serde(default)]
    pub env: Option<EnvSandbox>,

    #[serde(default)]
    pub tmpdir: Option<bool>,

    #[serde(default)]
    pub network: Option<NetworkSandbox>,

    /// Default scheduling priority for all commands (overridden per-command)
    #[serde(default)]
    pub priority: Option<Priority>,

    /// Default memory limit for all commands (overridden per-command)
    #[serde(default)]
    pub memory_limit: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EnvSandbox {
    Strict,
    Inherit,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NetworkSandbox {
    None,
    Host,
    Restricted,
}

/// A single input entry — flat array, discriminated by `type`
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Node {
    Env(EnvInput),
    File(FileInput),
    Binary(BinaryInput),
    Service(ServiceInput),
    Command(CommandInput),

    Platform(PlatformInput),
    Dns(DnsInput),
    Metric(MetricInput),
    Component(ComponentInput),
    Source(SourceInput),
    Std(StdInput),
    Flag(FlagInput),
}

impl Node {
    #[allow(dead_code)]
    pub fn phase(&self) -> Phase {
        match self {
            Node::Env(e) => e.phase.clone().unwrap_or(Phase::Seal),
            Node::File(f) => f.phase.clone().unwrap_or(Phase::Seal),
            Node::Binary(b) => b.phase.clone().unwrap_or(Phase::Build),
            Node::Service(s) => s.phase.clone().unwrap_or(Phase::Exec),
            Node::Command(c) => c.phase.clone().unwrap_or(Phase::Exec),

            Node::Platform(p) => p.phase.clone().unwrap_or(Phase::Build),
            Node::Dns(d) => d.phase.clone().unwrap_or(Phase::Exec),
            Node::Metric(m) => m.phase.clone().unwrap_or(Phase::Exec),
            Node::Component(_) => Phase::Seal,
            Node::Source(s) => s.phase.clone().unwrap_or(Phase::Seal),
            Node::Std(_) => Phase::Exec,
            Node::Flag(f) => f.phase.clone().unwrap_or(Phase::Exec),
        }
    }
}

/// Phase: when this input is evaluated
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Phase {
    /// Sealed at build time — verified during `besogne build`, embedded in binary
    Build,
    /// Precondition — checked at startup before execution
    Seal,
    /// Execution — part of the command DAG
    Exec,
}

// --- Native input types ---

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnvInput {
    /// Env var name. Defaults to the map key if omitted.
    #[serde(default)]
    pub name: Option<String>,

    #[serde(default)]
    pub value: Option<String>,

    #[serde(default)]
    pub from_env: Option<bool>,

    #[serde(default)]
    pub exists: Option<bool>,

    #[serde(default)]
    pub secret: Option<bool>,

    #[serde(default)]
    pub on_missing: Option<OnMissing>,

    #[serde(default)]
    pub phase: Option<Phase>,

    /// Merge strategy when this var already exists in scope.
    /// override (default): replace. prepend/append: join with separator.
    /// fallback: only set if not already present.
    #[serde(default)]
    pub merge: Option<EnvMerge>,

    /// Separator for prepend/append merge (default: ":")
    #[serde(default)]
    pub separator: Option<String>,

    /// Parent nodes in the DAG
    #[serde(default)]
    pub parents: Option<Vec<String>>,
}

/// Merge strategy for env nodes with existing values in scope
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum EnvMerge {
    /// Replace existing value (default)
    Override,
    /// Prepend new value before existing: new{sep}existing
    Prepend,
    /// Append new value after existing: existing{sep}new
    Append,
    /// Only set if not already present in scope
    Fallback,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileInput {
    pub path: String,

    #[serde(default)]
    pub expect: Option<String>,

    #[serde(default)]
    pub permissions: Option<String>,

    #[serde(default)]
    pub phase: Option<Phase>,

    /// Parent nodes in the DAG
    #[serde(default)]
    pub parents: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BinaryInput {
    /// Binary name for PATH resolution. Defaults to the map key if omitted.
    #[serde(default)]
    pub name: Option<String>,

    #[serde(default)]
    pub path: Option<String>,

    /// Expected version or semver constraint (e.g. "22", ">=1.22").
    #[serde(default)]
    pub version: Option<String>,

    /// Parent binary nodes this binary is embedded in (e.g. Go toolchain internals).
    #[serde(default)]
    pub parents: Option<Vec<String>>,

    #[serde(default)]
    pub phase: Option<Phase>,

    #[serde(default)]
    pub sealed: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceInput {
    #[serde(default)]
    pub tcp: Option<String>,

    #[serde(default)]
    pub http: Option<String>,

    #[serde(default)]
    pub phase: Option<Phase>,

    #[serde(default)]
    pub retry: Option<RetryConfig>,

    #[serde(default)]
    pub parents: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandInput {
    /// The action to perform
    pub run: ExecSpec,

    #[serde(default)]
    pub env: Option<HashMap<String, String>>,

    #[serde(default)]
    pub description: Option<String>,

    #[serde(default)]
    pub phase: Option<Phase>,

    /// Opt out of caching: always run, never skip (deploy, notify, etc.)
    #[serde(default)]
    pub side_effects: Option<bool>,

    /// Working directory for this command (relative to manifest dir).
    #[serde(default)]
    pub workdir: Option<String>,

    /// Parent nodes in the DAG — must complete before this command runs
    #[serde(default)]
    pub parents: Option<Vec<String>>,

    /// Extra args appended to `run` when --force is passed.
    #[serde(default)]
    pub force_args: Option<Vec<String>>,

    /// Extra args appended to `run` when --debug is passed.
    #[serde(default)]
    pub debug_args: Option<Vec<String>>,

    /// Retry configuration for transient failures
    #[serde(default)]
    pub retry: Option<RetryConfig>,

    /// Explicit idempotency verification toggle.
    /// - `true`: always verify on first run (even if >10s)
    /// - `false`: never verify
    /// - omitted (None): auto — verify if <10s, skip if >=10s
    #[serde(default)]
    pub verify: Option<bool>,

    /// Scheduling priority: "normal" (default), "low", "background".
    /// Controls CPU nice level + I/O scheduling priority.
    #[serde(default)]
    pub priority: Option<Priority>,

    /// Memory limit for this command (e.g. "2GB", "512MB").
    /// Process is killed if it exceeds this via RLIMIT_AS.
    #[serde(default)]
    pub memory_limit: Option<String>,

    /// Suppress live output streaming to terminal.
    /// Useful when stdout is consumed by a child source node.
    #[serde(default)]
    pub hide_output: Option<bool>,
}


#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlatformInput {
    #[serde(default)]
    pub os: Option<String>,

    #[serde(default)]
    pub arch: Option<String>,

    #[serde(default)]
    pub kernel_min: Option<String>,

    #[serde(default)]
    pub phase: Option<Phase>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DnsInput {
    pub host: String,

    #[serde(default)]
    pub expect: Option<String>,

    #[serde(default)]
    pub phase: Option<Phase>,

    #[serde(default)]
    pub retry: Option<RetryConfig>,

    #[serde(default)]
    pub parents: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricInput {
    pub metric: String,

    #[serde(default)]
    pub path: Option<String>,

    #[serde(default)]
    pub phase: Option<Phase>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComponentInput {
    /// Per-node overrides: node_name → partial node object to merge (replaces fields).
    #[serde(default)]
    pub overrides: Option<HashMap<String, serde_json::Value>>,

    /// Per-node array patches: node_name → field_name → { append, prepend, remove }.
    #[serde(default)]
    pub patch: Option<HashMap<String, HashMap<String, PatchOp>>>,
}

/// Array patch operations for component node fields.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PatchOp {
    #[serde(default)]
    pub append: Option<Vec<serde_json::Value>>,
    #[serde(default)]
    pub prepend: Option<Vec<serde_json::Value>>,
    #[serde(default)]
    pub remove: Option<Vec<serde_json::Value>>,
}

/// Source input — reads a map of env vars from a file or std parent.
/// The env vars are injected into commands that have this source as a parent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceInput {
    /// Parse format: "json", "dotenv", "shell"
    pub format: String,

    /// File to parse directly (alternative to reading from a std parent)
    #[serde(default)]
    pub path: Option<String>,

    /// Only keep these env var names (filter). If omitted, keep all.
    #[serde(default)]
    pub select: Option<Vec<String>>,

    #[serde(default)]
    pub phase: Option<Phase>,

    /// Parent nodes in the DAG
    #[serde(default)]
    pub parents: Option<Vec<String>>,
}

/// Std node — probe on command I/O (stdout, stderr, exit_code).
/// Always exec phase. Parent must be a command.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StdInput {
    /// Which stream: "stdout", "stderr", "exit_code"
    pub stream: String,

    /// Parent nodes (the command whose output to probe)
    #[serde(default)]
    pub parents: Option<Vec<String>>,

    /// Assert content contains these strings (for stdout/stderr)
    #[serde(default)]
    pub contains: Option<Vec<String>>,

    /// Assert exact match (for exit_code: "0")
    #[serde(default)]
    pub expect: Option<String>,
}

/// Flag node — a CLI flag that gates subtree execution via DAG parents.
/// Each flag+value combo is a separate node. Children only execute when
/// the flag matches the declared value.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlagInput {
    /// Flag name (the --long form). Defaults to node key.
    #[serde(default)]
    pub name: Option<String>,

    /// Short flag form (single char, e.g., 'n' for -n)
    #[serde(default)]
    pub short: Option<String>,

    /// Description shown in --help
    #[serde(default)]
    pub description: Option<String>,

    /// Value to match. For bool flags: true (when passed) or false (when not passed).
    /// For value flags: the string to match (e.g., "linux", "staging").
    #[serde(default)]
    pub value: Option<serde_json::Value>,

    #[serde(default)]
    pub phase: Option<Phase>,

    /// Parent nodes in the DAG
    #[serde(default)]
    pub parents: Option<Vec<String>>,
}

// --- Shared types ---

/// Exec specification — polymorphic (array, string, object)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ExecSpec {
    /// Array form: ["go", "test", "./..."]
    Array(Vec<String>),

    /// String form: "go test | grep PASS" (inline bash)
    Shell(String),

    /// Script form: { "file": "./run.sh", "args": ["--flag"] }
    Script {
        file: String,
        #[serde(default)]
        args: Option<Vec<String>>,
    },
}


#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetryConfig {
    /// Maximum number of attempts (including the first try)
    pub attempts: u32,

    /// Base interval between retries (e.g. "1s", "500ms", "2m")
    pub interval: String,

    /// Backoff strategy: "fixed", "linear", "exponential" (default: "fixed")
    #[serde(default)]
    pub backoff: Option<String>,

    /// Maximum interval cap when using backoff (e.g. "30s", "5m")
    #[serde(default)]
    pub max_interval: Option<String>,

    /// Total timeout for all attempts combined (e.g. "5m", "10m")
    #[serde(default)]
    pub timeout: Option<String>,
}

/// Scheduling priority for commands.
/// Controls CPU nice level + I/O scheduling class.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Priority {
    /// Default scheduling — no changes
    Normal,
    /// Reduced priority: nice 10 + best-effort I/O
    Low,
    /// Lowest priority: nice 19 + idle I/O
    Background,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OnMissing {
    /// Default: resource MUST exist, abort if missing.
    Fail,
    /// Skip this node AND all children transitively.
    /// The sub-DAG rooted here is disabled.
    /// Use for: CI-only steps, platform-specific features.
    Skip,
    /// Succeed with null value, children execute normally.
    /// Absent value is part of cache key — if value appears later, cache invalidates.
    /// Use for: system env vars (POSIXLY_CORRECT, LANG, TERM).
    Continue,
}


