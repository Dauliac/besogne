use crate::manifest;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Content-addressed ID: type:identifier:blake3_short
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct ContentId(pub String);

impl ContentId {
    #[allow(dead_code)]
    pub fn new(type_name: &str, identifier: &str, hash: &[u8]) -> Self {
        let short_hash = &blake3::Hash::from(
            <[u8; 32]>::try_from(hash).unwrap_or([0; 32]),
        ).to_hex()[..8];
        ContentId(format!("{type_name}:{identifier}:{short_hash}"))
    }

    pub fn from_content(type_name: &str, identifier: &str, content: &[u8]) -> Self {
        let hash = blake3::hash(content);
        let short = &hash.to_hex()[..8];
        ContentId(format!("{type_name}:{identifier}:{short}"))
    }
}

impl std::fmt::Display for ContentId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// The compiled intermediate representation — embedded in the besogne binary
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BesogneIR {
    pub metadata: Metadata,
    pub sandbox: SandboxResolved,
    #[serde(default)]
    pub flags: Vec<ResolvedFlag>,
    pub nodes: Vec<ResolvedNode>,
}

/// A resolved user-defined flag
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolvedFlag {
    pub name: String,
    #[serde(default)]
    pub short: Option<char>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub doc: Option<String>,
    pub kind: ResolvedFlagKind,
    #[serde(default)]
    pub default: Option<serde_json::Value>,
    #[serde(default)]
    pub values: Option<Vec<String>>,
    #[serde(default)]
    pub required: bool,
    pub env_var: String,
    #[serde(default)]
    pub subcommand: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ResolvedFlagKind {
    Bool,
    String,
    Positional,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Metadata {
    pub name: String,
    pub version: String,
    pub description: String,
    /// Absolute path where the manifest was found — commands run relative to this
    #[serde(default)]
    pub workdir: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SandboxResolved {
    pub env: EnvSandboxResolved,
    pub tmpdir: bool,
    pub network: NetworkSandboxResolved,
    /// Default scheduling priority for all commands
    #[serde(default)]
    pub priority: PriorityResolved,
    /// Default memory limit in bytes for all commands (None = unlimited)
    #[serde(default)]
    pub memory_limit: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum EnvSandboxResolved {
    Strict,
    Inherit,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum NetworkSandboxResolved {
    None,
    Host,
    Restricted,
}

/// A resolved input — content-hashed, ready for evaluation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolvedNode {
    pub id: ContentId,
    pub phase: manifest::Phase,
    pub node: ResolvedNativeNode,

    /// Parent inputs in the DAG — must complete before this input runs
    #[serde(default)]
    pub parents: Vec<ContentId>,

    /// Traceability: which component produced this
    #[serde(default)]
    pub from_component: Option<String>,

    /// Build-time sealed data (if phase=build)
    #[serde(default)]
    pub sealed: Option<SealedSnapshot>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SealedSnapshot {
    pub hash: String,
    pub size: Option<u64>,
}

/// Detected source of a binary — compile-time variant polymorphism.
/// Each variant carries source-specific metadata for safe version extraction.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum BinarySourceResolved {
    /// Binary lives in the Nix store — version parsed from store path, immutable
    Nix {
        store_path: String,
        #[serde(default)]
        pname: Option<String>,
    },
    /// Binary managed by mise — version parsed from install path
    Mise {
        tool: String,
    },
    /// System binary — no safe version detection by default
    System,
}

/// Native input types in the IR — resolved and ready
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum ResolvedNativeNode {
    Env {
        name: String,
        #[serde(default)]
        value: Option<String>,
        #[serde(default)]
        secret: bool,
        /// How to handle missing env var: fail (default), skip (propagate), continue (null hash).
        #[serde(default)]
        on_missing: OnMissingResolved,
    },
    File {
        path: String,
        #[serde(default)]
        expect: Option<String>,
        #[serde(default)]
        permissions: Option<String>,
    },
    Binary {
        name: String,
        /// Explicit path from manifest
        #[serde(default)]
        path: Option<String>,
        #[serde(default)]
        version_constraint: Option<String>,
        /// Parent binary names — this binary is embedded in (e.g. Go's `compile` inside `go`).
        /// Skips PATH resolution; hash derived from parent(s).
        #[serde(default)]
        parents: Vec<String>,
        /// Build-time resolved: detected source (Nix/Mise/System)
        #[serde(default)]
        source: Option<BinarySourceResolved>,
        /// Build-time resolved: canonical absolute path
        #[serde(default)]
        resolved_path: Option<String>,
        /// Build-time resolved: version (safe extraction from source, or --version if requested)
        #[serde(default)]
        resolved_version: Option<String>,
        /// Build-time resolved: BLAKE3 hash of binary content
        #[serde(default)]
        binary_hash: Option<String>,
    },
    Service {
        #[serde(default)]
        name: Option<String>,
        #[serde(default)]
        tcp: Option<String>,
        #[serde(default)]
        http: Option<String>,
        #[serde(default)]
        retry: Option<RetryResolved>,
    },
    Command {
        name: String,
        run: Vec<String>,
        #[serde(default)]
        env: HashMap<String, String>,
        #[serde(default)]
        side_effects: bool,
        #[serde(default)]
        workdir: Option<String>,
        #[serde(default)]
        force_args: Vec<String>,
        #[serde(default)]
        debug_args: Vec<String>,
        #[serde(default)]
        retry: Option<RetryResolved>,
        /// Explicit idempotency toggle: true=always, false=never, None=auto (<10s)
        #[serde(default)]
        verify: Option<bool>,
        /// Resource limits (priority + memory cap)
        #[serde(default)]
        resources: ResourceLimits,
    },

    Platform {
        #[serde(default)]
        os: Option<String>,
        #[serde(default)]
        arch: Option<String>,
    },
    Dns {
        host: String,
        #[serde(default)]
        expect: Option<String>,
        #[serde(default)]
        retry: Option<RetryResolved>,
    },
    Metric {
        metric: String,
        #[serde(default)]
        path: Option<String>,
    },
    Source {
        /// Parse format: "json", "dotenv", "shell"
        format: String,
        /// File to parse directly (if no std parent)
        #[serde(default)]
        path: Option<String>,
        /// Only keep these env var names
        #[serde(default)]
        select: Option<Vec<String>>,
        /// Build-time sealed env map (if phase=build)
        #[serde(default)]
        sealed_env: Option<HashMap<String, String>>,
    },
    Std {
        stream: String,
        #[serde(default)]
        contains: Vec<String>,
        #[serde(default)]
        expect: Option<String>,
    },
}

impl ResolvedNativeNode {
    /// Persistent nodes exist in the real world and CAN drift externally.
    /// Ephemeral nodes (std) exist only in besogne's cache and cannot drift.
    pub fn is_persistent(&self) -> bool {
        !matches!(self, ResolvedNativeNode::Std { .. })
    }
}

/// Resolved retry configuration — durations parsed from strings
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetryResolved {
    pub attempts: u32,
    /// Base interval in milliseconds
    pub interval_ms: u64,
    pub backoff: RetryBackoff,
    /// Maximum interval cap in milliseconds (None = unlimited)
    #[serde(default)]
    pub max_interval_ms: Option<u64>,
    /// Total timeout in milliseconds (None = unlimited)
    #[serde(default)]
    pub timeout_ms: Option<u64>,
}

/// Resolved scheduling priority — platform-independent
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum PriorityResolved {
    #[default]
    Normal,
    Low,
    Background,
}

/// Resolved resource limits for a command
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ResourceLimits {
    /// Scheduling priority
    #[serde(default)]
    pub priority: PriorityResolved,
    /// Memory limit in bytes (RLIMIT_AS). None = unlimited.
    #[serde(default)]
    pub memory_limit: Option<u64>,
}

/// How to handle a missing resource at probe time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum OnMissingResolved {
    /// Resource MUST exist. Probe fails → execution aborts.
    #[default]
    Fail,
    /// Skip this node AND all children transitively. Sub-DAG disabled.
    Skip,
    /// Succeed with null hash. Children execute normally.
    /// Cache invalidates if value appears in future run.
    Continue,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RetryBackoff {
    Fixed,
    Linear,
    Exponential,
}

impl RetryResolved {
    /// Compute the delay for a given attempt (0-indexed)
    pub fn delay_for_attempt(&self, attempt: u32) -> std::time::Duration {
        let base = self.interval_ms;
        let ms = match self.backoff {
            RetryBackoff::Fixed => base,
            RetryBackoff::Linear => base * (attempt as u64 + 1),
            RetryBackoff::Exponential => base * 2u64.saturating_pow(attempt),
        };
        let capped = match self.max_interval_ms {
            Some(max) => ms.min(max),
            None => ms,
        };
        std::time::Duration::from_millis(capped)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn test_fixed_backoff() {
        let r = RetryResolved {
            attempts: 5,
            interval_ms: 1000,
            backoff: RetryBackoff::Fixed,
            max_interval_ms: None,
            timeout_ms: None,
        };
        assert_eq!(r.delay_for_attempt(0), Duration::from_secs(1));
        assert_eq!(r.delay_for_attempt(3), Duration::from_secs(1));
    }

    #[test]
    fn test_linear_backoff() {
        let r = RetryResolved {
            attempts: 5,
            interval_ms: 1000,
            backoff: RetryBackoff::Linear,
            max_interval_ms: None,
            timeout_ms: None,
        };
        assert_eq!(r.delay_for_attempt(0), Duration::from_secs(1));  // 1000 * 1
        assert_eq!(r.delay_for_attempt(1), Duration::from_secs(2));  // 1000 * 2
        assert_eq!(r.delay_for_attempt(4), Duration::from_secs(5));  // 1000 * 5
    }

    #[test]
    fn test_exponential_backoff() {
        let r = RetryResolved {
            attempts: 5,
            interval_ms: 1000,
            backoff: RetryBackoff::Exponential,
            max_interval_ms: None,
            timeout_ms: None,
        };
        assert_eq!(r.delay_for_attempt(0), Duration::from_secs(1));  // 1000 * 2^0
        assert_eq!(r.delay_for_attempt(1), Duration::from_secs(2));  // 1000 * 2^1
        assert_eq!(r.delay_for_attempt(3), Duration::from_secs(8));  // 1000 * 2^3
    }

    #[test]
    fn test_max_interval_cap() {
        let r = RetryResolved {
            attempts: 10,
            interval_ms: 1000,
            backoff: RetryBackoff::Exponential,
            max_interval_ms: Some(5000),
            timeout_ms: None,
        };
        assert_eq!(r.delay_for_attempt(0), Duration::from_secs(1));
        assert_eq!(r.delay_for_attempt(3), Duration::from_secs(5));  // 8000 capped to 5000
        assert_eq!(r.delay_for_attempt(10), Duration::from_secs(5)); // capped
    }
}
