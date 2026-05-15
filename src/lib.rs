pub mod adopt;
pub mod compile;
pub mod error;
pub mod event;
pub mod facade;
pub mod ir;
pub mod manifest;
#[allow(dead_code)]
pub mod output;
pub mod probe;
pub mod runtime;
pub mod tracer;

// Re-export the facade as the primary public API
pub use facade::{Besogne, BesogneBuilder, BuildOutput, RunOutput, CommandSummary, CheckOutput, ManifestInfo};
pub use error::{BesogneError, Result};
pub use event::{BesogneEvent, EventHandler};
pub use runtime::cli::{LogFormat, RuntimeConfig};
