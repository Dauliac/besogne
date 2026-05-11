use crate::manifest;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Content-addressed ID: type:identifier:blake3_short
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct ContentId(pub String);

impl ContentId {
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

    /// Traceability: which plugin produced this
    #[serde(default)]
    pub from_plugin: Option<String>,

    /// Build-time sealed data (if phase=build)
    #[serde(default)]
    pub sealed: Option<SealedSnapshot>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SealedSnapshot {
    pub hash: String,
    pub size: Option<u64>,
    pub verified_at: String,
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
        expect: Option<String>,
        #[serde(default)]
        secret: bool,
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
    },
    Command {
        name: String,
        /// The action to perform (was: exec)
        run: Vec<String>,
        #[serde(default)]
        env: HashMap<String, String>,
        /// Postconditions — what must be true after this command
        #[serde(default)]
        postconditions: Vec<manifest::PostconditionSpec>,
        /// Opt out of caching: always run, never skip
        #[serde(default)]
        side_effects: bool,
        /// Output assertions (opt-in)
        #[serde(default)]
        output: Option<manifest::OutputSpec>,
        /// Per-command working directory (absolute). If None, uses metadata.workdir.
        #[serde(default)]
        workdir: Option<String>,
        /// Extra args appended to `run` when --force is passed (tool cache invalidation).
        #[serde(default)]
        force_args: Vec<String>,
        /// Extra args appended to `run` when --debug is passed (verbose/debug output).
        #[serde(default)]
        debug_args: Vec<String>,
    },
    User {
        #[serde(default)]
        in_group: Option<String>,
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
    },
    Metric {
        metric: String,
        #[serde(default)]
        path: Option<String>,
    },
}
