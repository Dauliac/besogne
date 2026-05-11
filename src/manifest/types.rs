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
    pub inputs: HashMap<String, Input>,
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
pub enum Input {
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
}

impl Input {
    /// Get the phase for this input (build/pre/exec)
    pub fn phase(&self) -> Phase {
        match self {
            Input::Env(e) => e.phase.clone().unwrap_or(Phase::Pre),
            Input::File(f) => f.phase.clone().unwrap_or(Phase::Pre),
            Input::Binary(b) => b.phase.clone().unwrap_or(Phase::Build),
            Input::Service(s) => s.phase.clone().unwrap_or(Phase::Pre),
            Input::Command(c) => c.phase.clone().unwrap_or(Phase::Exec),
            Input::User(u) => u.phase.clone().unwrap_or(Phase::Pre),
            Input::Platform(p) => p.phase.clone().unwrap_or(Phase::Build),
            Input::Dns(d) => d.phase.clone().unwrap_or(Phase::Pre),
            Input::Metric(m) => m.phase.clone().unwrap_or(Phase::Pre),
            Input::Plugin(_) => Phase::Pre,
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
    Pre,
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
    pub expect: Option<String>,

    #[serde(default)]
    pub value: Option<String>,

    #[serde(default)]
    pub from_env: Option<bool>,

    #[serde(default)]
    pub exists: Option<bool>,

    #[serde(default)]
    pub secret: Option<bool>,

    #[serde(default)]
    pub values: Option<Vec<String>>,

    #[serde(default)]
    pub min: Option<serde_json::Value>,

    #[serde(default)]
    pub max: Option<serde_json::Value>,

    #[serde(default)]
    pub pattern: Option<String>,

    #[serde(default)]
    pub on_missing: Option<OnMissing>,

    #[serde(default)]
    pub phase: Option<Phase>,

    #[serde(default)]
    pub validate: Option<HashMap<String, serde_json::Value>>,

    #[serde(default)]
    pub extract: Option<ExtractConfig>,
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

    #[serde(default)]
    pub validate: Option<HashMap<String, serde_json::Value>>,

    #[serde(default)]
    pub extract: Option<ExtractConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BinaryInput {
    /// Binary name for PATH resolution. Defaults to the map key if omitted.
    #[serde(default)]
    pub name: Option<String>,

    #[serde(default)]
    pub path: Option<String>,

    /// Expected version or semver constraint (e.g. "22", ">=1.22").
    /// For System binaries, setting this enables --version probing.
    /// For Nix/Mise binaries, version is always extracted safely from path.
    #[serde(default)]
    pub version: Option<String>,

    #[serde(default)]
    pub phase: Option<Phase>,

    #[serde(default)]
    pub sealed: Option<bool>,

    #[serde(default)]
    pub validate: Option<HashMap<String, serde_json::Value>>,

    #[serde(default)]
    pub metadata: Option<HashMap<String, serde_json::Value>>,
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
    pub validate: Option<HashMap<String, serde_json::Value>>,

    #[serde(default)]
    pub after: Option<Vec<String>>,
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

    #[serde(default)]
    /// Opt out of caching: always run, never skip (deploy, notify, etc.)
    pub side_effects: Option<bool>,

    /// Postconditions — what must be true after this command (was: outputs)
    #[serde(default)]
    pub ensure: Option<Vec<EnsureSpec>>,

    /// Ordering constraints (was: dependencies)
    #[serde(default)]
    pub after: Option<Vec<String>>,

    #[serde(default)]
    pub validate: Option<HashMap<String, serde_json::Value>>,

    #[serde(default)]
    pub extract: Option<ExtractConfig>,

    /// Output assertions — validate command stdout/stderr/exit_code.
    /// Opt-in: disabled by default (output is non-deterministic in general).
    #[serde(default)]
    pub output: Option<OutputSpec>,
}

/// Specification for command output validation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutputSpec {
    /// Expected exit code (default: 0)
    #[serde(default)]
    pub exit_code: Option<i32>,

    /// Assert stdout contains this string
    #[serde(default)]
    pub stdout_contains: Option<Vec<String>>,

    /// Assert stderr contains this string
    #[serde(default)]
    pub stderr_contains: Option<Vec<String>>,

    /// Assert stdout matches this regex
    #[serde(default)]
    pub stdout_matches: Option<String>,

    /// Assert stderr matches this regex
    #[serde(default)]
    pub stderr_matches: Option<String>,

    /// Parse stdout as JSON and assert it matches (json path → expected value)
    #[serde(default)]
    pub json: Option<HashMap<String, serde_json::Value>>,
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

    #[serde(default)]
    pub validate: Option<HashMap<String, serde_json::Value>>,
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

// --- Shared types ---

/// Exec specification — polymorphic (array, string, object)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ExecSpec {
    /// Array form: ["go", "test", "./..."]
    Array(Vec<String>),

    /// String form: "go test | grep PASS" (inline bash)
    Shell(String),

    /// Pipe form: { "pipe": [["go", "test"], ["tail", "-1"]] }
    Pipe {
        pipe: Vec<Vec<String>>,
    },

    /// Script form: { "file": "./run.sh", "args": ["--flag"] }
    Script {
        file: String,
        #[serde(default)]
        args: Option<Vec<String>>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractConfig {
    pub format: String,
    pub fields: HashMap<String, String>,
}

/// Postcondition spec (was: OutputSpec)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnsureSpec {
    #[serde(rename = "type")]
    pub ensure_type: String,

    pub path: String,

    #[serde(default)]
    pub expect: Option<String>,

    #[serde(default)]
    pub required: Option<bool>,
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
