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

    /// Plugin sources: namespace → source.
    /// "builtin" for embedded plugins, "./path" for local, "github:org/repo#ref" for remote.
    #[serde(default)]
    pub plugins: HashMap<String, String>,

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
    User(UserInput),
    Platform(PlatformInput),
    Dns(DnsInput),
    Metric(MetricInput),
    Plugin(PluginInput),
    Source(SourceInput),
}

impl Node {
    /// Get the phase for this input (build/pre/exec)
    pub fn phase(&self) -> Phase {
        match self {
            Node::Env(e) => e.phase.clone().unwrap_or(Phase::Seal),
            Node::File(f) => f.phase.clone().unwrap_or(Phase::Seal),
            Node::Binary(b) => b.phase.clone().unwrap_or(Phase::Build),
            Node::Service(s) => s.phase.clone().unwrap_or(Phase::Seal),
            Node::Command(c) => c.phase.clone().unwrap_or(Phase::Exec),
            Node::User(u) => u.phase.clone().unwrap_or(Phase::Seal),
            Node::Platform(p) => p.phase.clone().unwrap_or(Phase::Build),
            Node::Dns(d) => d.phase.clone().unwrap_or(Phase::Seal),
            Node::Metric(m) => m.phase.clone().unwrap_or(Phase::Seal),
            Node::Plugin(_) => Phase::Seal,
            Node::Source(s) => s.phase.clone().unwrap_or(Phase::Seal),
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
    pub on_fail: Option<OnFail>,

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

    #[serde(default)]
    pub on_fail: Option<OnFail>,

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
}


#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserInput {
    #[serde(default)]
    pub in_group: Option<String>,

    #[serde(default)]
    pub not: Option<String>,

    #[serde(default)]
    pub phase: Option<Phase>,
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
pub struct PluginInput {
    pub plugin: String,

    #[serde(default)]
    pub overrides: Option<HashMap<String, serde_json::Value>>,

    /// All remaining fields are plugin params
    #[serde(flatten)]
    pub params: HashMap<String, serde_json::Value>,
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
    pub attempts: u32,
    pub interval: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OnMissing {
    Fail,
    Skip,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OnFail {
    Fail,
    Skip,
    Warn,
}
