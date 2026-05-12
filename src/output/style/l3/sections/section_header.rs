//! SectionHeader: ── name ────────────────
//! Axes: structure x weight::L1 (name) + weight::L3 (rule).

use crate::output::style::{bold, dim, weight, palette::RESET};

pub fn render(name: &str) -> String {
    let rule = "\u{2500}".repeat(50 - name.len().min(48));
    format!("{}\u{2500}\u{2500} {}{RESET} {}\u{2500}{RESET}",
        weight::L3, bold(name), dim(&rule))
}
