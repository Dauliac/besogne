//! Views — logical compositions of L3 components per context.
//!
//! A view is a distinct output mode: a unique arrangement of sections and components.
//! Views import ONLY from `style::l3::` (atoms, items, sections, telemetry).
//! They bridge domain types (IR, cache) to presentation (L3 components).
//!
//! ```text
//! Domain (IR, Cache, ProbeResult)
//!   ↓ extracted by
//! Views (build, run, status, ...)
//!   ↓ composed from
//! L3 Components (atoms → items → sections, telemetry)
//!   ↓ styled with
//! L2 Semantic Tokens (phase, outcome, weight, ...)
//!   ↓ rendered via
//! L1 Palette (raw ANSI)
//! ```

pub mod build;
pub mod run;
pub mod status;
pub mod list;
pub mod check;
pub mod dump;
