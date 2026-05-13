use std::fmt;
use std::path::PathBuf;

/// Typed error enum for all besogne operations.
///
/// Each variant carries structured context so callers can pattern-match
/// on the error domain without parsing strings. Display produces the
/// same user-facing diagnostics as before (DiagBuilder formatting).
#[derive(Debug)]
pub enum BesogneError {
    /// Manifest parsing / validation errors
    Manifest(String),

    /// Component expansion errors (recursive, overrides, patches)
    Component(String),

    /// Lowering / IR compilation errors (type resolution, DAG wiring, validation)
    Compile(String),

    /// Binary embedding / extraction errors
    Embed(String),

    /// DAG construction / cycle detection errors
    Dag(String),

    /// Binary probe / resolution errors
    Probe(String),

    /// Tracer / command execution errors
    Tracer(String),

    /// Cache I/O errors
    Cache(String),

    /// Config loading errors
    Config(String),

    /// Source file parsing errors (dotenv, json, shell)
    Source(String),

    /// Adopt pipeline errors (package.json → besogne.toml)
    Adopt(String),

    /// CLI / manifest discovery errors
    Cli(String),

    /// Multiple errors collected from a validation pass
    Multi(Vec<BesogneError>),

    /// I/O error with path context
    Io {
        path: PathBuf,
        cause: std::io::Error,
    },
}

impl fmt::Display for BesogneError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Manifest(msg)
            | Self::Component(msg)
            | Self::Compile(msg)
            | Self::Embed(msg)
            | Self::Dag(msg)
            | Self::Probe(msg)
            | Self::Tracer(msg)
            | Self::Cache(msg)
            | Self::Config(msg)
            | Self::Source(msg)
            | Self::Adopt(msg)
            | Self::Cli(msg) => write!(f, "{msg}"),
            Self::Multi(errors) => {
                for (i, e) in errors.iter().enumerate() {
                    if i > 0 {
                        write!(f, "\n\n")?;
                    }
                    write!(f, "{e}")?;
                }
                Ok(())
            }
            Self::Io { path, cause } => write!(f, "{}: {cause}", path.display()),
        }
    }
}

impl std::error::Error for BesogneError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { cause, .. } => Some(cause),
            _ => None,
        }
    }
}

/// Convenience alias used throughout the codebase.
pub type Result<T> = std::result::Result<T, BesogneError>;
