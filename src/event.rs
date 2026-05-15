//! Event system for besogne — decouples execution from presentation.
//!
//! The [`EventHandler`] trait is the primary extension point for consumers
//! who want structured access to build/run lifecycle events without parsing
//! terminal output.
//!
//! # Example
//!
//! ```no_run
//! use besogne::event::{BesogneEvent, EventHandler};
//!
//! struct MyHandler;
//!
//! impl EventHandler for MyHandler {
//!     fn on_event(&mut self, event: &BesogneEvent<'_>) {
//!         match event {
//!             BesogneEvent::CommandEnd { name, exit_code, wall_ms, .. } => {
//!                 println!("{name} finished with code {exit_code} in {wall_ms}ms");
//!             }
//!             BesogneEvent::Summary { exit_code, wall_ms } => {
//!                 println!("Done: exit={exit_code} wall={wall_ms}ms");
//!             }
//!             _ => {}
//!         }
//!     }
//! }
//! ```

use crate::ir::{BesogneIR, ResolvedNode};
use crate::output::{CommandContext, ProbeStatus};
use crate::probe::ProbeResult;
use crate::runtime::cache::CachedCommand;
use crate::tracer::CommandResult;

/// Every observable event during a besogne build or run.
///
/// Events are emitted in chronological order. Consumers can pattern-match
/// on the variants they care about and ignore the rest.
#[derive(Debug)]
#[non_exhaustive]
pub enum BesogneEvent<'a> {
    // ── Lifecycle ──

    /// Run started. Emitted once at the beginning.
    Start {
        ir: &'a BesogneIR,
    },

    /// A phase (seal, exec) has started.
    PhaseStart {
        phase: &'a str,
        node_count: usize,
    },

    /// A phase has ended.
    PhaseEnd {
        phase: &'a str,
    },

    // ── Probes ──

    /// A probe (seal or exec phase) completed.
    ProbeResult {
        node: &'a ResolvedNode,
        result: &'a ProbeResult,
        status: ProbeStatus,
    },

    /// Probes that changed since the last run.
    ChangedProbes {
        names: &'a [String],
    },

    /// Build-phase pinned binary summary.
    BuildPinnedSummary {
        nodes: &'a [&'a ResolvedNode],
    },

    // ── Commands ──

    /// A command is about to execute.
    CommandStart {
        name: &'a str,
        exec: &'a [String],
        ctx: &'a CommandContext<'a>,
    },

    /// Streaming output from a running command.
    CommandOutput {
        name: &'a str,
        stdout: &'a str,
        stderr: &'a str,
    },

    /// A command finished executing.
    CommandEnd {
        name: &'a str,
        exit_code: i32,
        wall_ms: u64,
        result: &'a CommandResult,
    },

    /// A command was skipped (served from cache).
    CommandCached {
        name: &'a str,
        exec: &'a [String],
        cached: &'a CachedCommand,
        ctx: &'a CommandContext<'a>,
    },

    // ── Diagnostics ──

    /// Undeclared dependencies detected.
    UndeclaredDeps {
        binaries: &'a [String],
        env_vars: &'a [String],
    },

    // ── Skip / Summary ──

    /// Entire run skipped (all inputs cached, nothing changed).
    Skip {
        total_nodes: usize,
        ran_at: &'a str,
        duration_ms: u64,
    },

    /// Final summary. Always the last event emitted.
    Summary {
        exit_code: i32,
        wall_ms: u64,
    },
}

/// Trait for receiving structured besogne events.
///
/// Implement this trait to build custom integrations (CI reporters,
/// IDE plugins, metrics collectors, etc.) without parsing terminal output.
pub trait EventHandler: Send {
    /// Called for each event during build or run.
    fn on_event(&mut self, event: &BesogneEvent<'_>);
}

/// Blanket impl: any `FnMut(&BesogneEvent)` is an EventHandler.
impl<F: FnMut(&BesogneEvent<'_>) + Send> EventHandler for F {
    fn on_event(&mut self, event: &BesogneEvent<'_>) {
        self(event);
    }
}
