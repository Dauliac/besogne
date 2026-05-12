//! StatusView — `--status` inspection mode.
//!
//! No live execution. Shows cached state of the last run.
//! Everything is [temporality=cached/static] → default L3 (dim).
//! Only section headers and failures break out of dim.
//!
//! Sections:
//!   1. Header (manifest name + hash)
//!   2. Execution (unified trace: DAG → output → processes → metrics)
//!   3. Diagnostic (warnings/errors from last run, optional)
//!
//! L3 components used:
//!   sections::section_header
//!   sections::diag_block
//!   telemetry::execution_tree  — the entire unified tree

use crate::ir::BesogneIR;
use crate::runtime::cache::ContextCache;
use crate::output::style::{bold, dim};
use crate::output::style::l3;

/// Render the full status view.
pub fn render(ir: &BesogneIR, cache: &ContextCache) {
    // ── Header ──
    eprintln!("{} {} — {}",
        bold(&ir.metadata.name),
        dim(&format!("v{}", ir.metadata.version)),
        dim(&ir.metadata.description));
    eprintln!();

    // ── Execution ── (unified tree: phases → nodes → output → processes → metrics)
    eprintln!("{}", l3::sections::section_header::render("execution"));
    l3::telemetry::execution_tree::render(ir, cache);
    eprintln!();

    // ── Diagnostic (optional) ──
    if !cache.non_idempotent.is_empty() {
        eprintln!("{}", l3::sections::section_header::render("diagnostic"));
        eprintln!("  {}", l3::sections::diag_block::warning(
            &format!("{} non-idempotent command(s): {}",
                cache.non_idempotent.len(),
                cache.non_idempotent.join(", "))));
        eprintln!("    {}", dim("add side_effects = true if intentional"));
        eprintln!();
    }
}
