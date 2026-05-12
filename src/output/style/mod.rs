//! Design token system for terminal output.
//!
//! Five orthogonal axes govern every rendered element:
//!
//!   Axis 1 — **Domain**: what KIND of data (identity, state, output, telemetry, structure, diagnostic)
//!   Axis 2 — **Phase**: WHERE in the pipeline (build → magenta, seal → blue, exec → cyan)
//!   Axis 3 — **Outcome**: WHAT happened (ok → green, warn → yellow, fail → red)
//!   Axis 4 — **Temporality**: WHEN (live → L2, cached → L3, static → L3)
//!   Axis 5 — **Weight**: HOW important (L1 bold, L2 normal, L3 dim)
//!
//! Token layers:
//!   L1 (`l1/`) — Palette: raw ANSI codes, no semantics
//!   L2 (`l2/`) — Semantic: axis tokens mapped to palette entries
//!   L3 (`l3/`) — Component: pure rendering functions consuming L2 tokens

mod l1;
mod l2;
pub mod l3;

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
//  L1 — Palette
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

pub use l1::palette;

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
//  L2 — Semantic axes
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

pub use l2::phase;
pub use l2::outcome;
pub use l2::weight;
pub use l2::temporality;

// L2 — Cross-axis composites
pub use l2::status;

// L2 — Domain-scoped color tokens
pub use l2::node;
pub use l2::telemetry;
pub use l2::ptree;
pub use l2::diagnostic;

// L2 — Text tokens
pub use l2::label;
pub use l2::badge;
pub use l2::phase_label;
pub use l2::metric_label;
pub use l2::icon;
pub use l2::layout;
pub use l2::message;

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
//  L3 — Component re-exports (convenience)
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

pub use l3::sections::diag_block::DiagBuilder;

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
//  Utilities (thin wrappers over palette — used by L3 components and renderers)
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

use palette::RESET;

/// Wrap text in a semantic token color, auto-resetting.
pub fn styled(token: &str, text: &str) -> String {
    if token.is_empty() {
        text.to_string()
    } else {
        format!("{token}{text}{RESET}")
    }
}

/// Weight L3: wrap text in dim.
pub fn dim(text: &str) -> String {
    format!("{}{text}{RESET}", weight::L3)
}

/// Weight L1: wrap text in bold.
pub fn bold(text: &str) -> String {
    format!("{}{text}{RESET}", weight::L1)
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
//  Legacy function aliases — delegate to L3 components
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

pub fn exit_code(code: i32) -> String { l3::atoms::exit_code::render(code) }
pub fn status_badge(label: &str, token: &str) -> String { l3::atoms::status_badge::render(label, token) }
pub fn section_header(name: &str) -> String { l3::sections::section_header::render(name) }
pub fn error_diag(msg: &str) -> String { l3::sections::diag_block::error(msg) }
pub fn warning_diag(msg: &str) -> String { l3::sections::diag_block::warning(msg) }
pub fn diag_hint(msg: &str) -> String { l3::sections::diag_block::hint(msg) }
pub fn diag_error(msg: &str) -> String { l3::sections::diag_block::error(msg) }
pub fn diag_warning(msg: &str) -> String { l3::sections::diag_block::warning(msg) }
