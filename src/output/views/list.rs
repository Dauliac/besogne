//! ListView — `besogne list` output.
//!
//! Shows discovered manifests with names and descriptions.
//! Simple view, minimal L3 composition.
//!
//! L3 components used:
//!   atoms::node_badge (optional, for node counts)

use crate::output::style::{bold, dim};

pub struct ManifestEntry<'a> {
    pub name: &'a str,
    pub description: &'a str,
    pub path: &'a str,
    pub node_count: usize,
    pub command_count: usize,
    pub component_count: usize,
}

/// Render compact list (default).
pub fn render(entries: &[ManifestEntry]) {
    for e in entries {
        eprintln!("  {:<16}{}", bold(e.name), e.description);
    }
}

/// Render verbose list (--verbose).
pub fn render_verbose(entries: &[ManifestEntry]) {
    for e in entries {
        eprintln!("  {}", dim(e.path));
        eprintln!("    {}", e.description);
        eprintln!("    nodes: {} ({} commands, {} components)",
            e.node_count, e.command_count, e.component_count);
        eprintln!();
    }
}
