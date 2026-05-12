//! DumpView — `--dump` / `--dump-internal` output.
//!
//! Shows IR content for debugging.
//! Two modes: human-readable summary, or raw JSON.
//! Minimal L3 composition — mostly plain text.

use crate::ir::BesogneIR;
use crate::ir::ResolvedNativeNode;
use crate::manifest::Phase;
use crate::output::style::{bold, dim};

/// Render human-readable IR dump.
pub fn render_human(ir: &BesogneIR) {
    println!("{} v{}", bold(&ir.metadata.name), ir.metadata.version);
    println!("{}", ir.metadata.description);
    println!();

    let build: Vec<_> = ir.nodes.iter().filter(|n| n.phase == Phase::Build).collect();
    let seal: Vec<_> = ir.nodes.iter().filter(|n| n.phase == Phase::Seal).collect();
    let exec: Vec<_> = ir.nodes.iter().filter(|n| n.phase == Phase::Exec).collect();

    if !build.is_empty() {
        println!("Build phase ({}):", build.len());
        for n in &build { println!("  {}", dim(&n.id.to_string())); }
        println!();
    }
    if !seal.is_empty() {
        println!("Seal phase ({}):", seal.len());
        for n in &seal { println!("  {}", dim(&n.id.to_string())); }
        println!();
    }
    if !exec.is_empty() {
        println!("Exec phase ({}):", exec.len());
        for n in &exec { println!("  {}", dim(&n.id.to_string())); }
        println!();
    }

    let se_count = ir.nodes.iter().filter(|n| matches!(&n.node,
        ResolvedNativeNode::Command { side_effects: true, .. }
    )).count();
    if se_count > 0 {
        println!("Side effects: {se_count} command(s) always run");
    }
}

/// Render raw JSON IR dump.
pub fn render_json(ir: &BesogneIR) {
    let json = serde_json::to_string_pretty(ir)
        .unwrap_or_else(|e| format!("{{\"error\": \"{e}\"}}"));
    println!("{json}");
}
