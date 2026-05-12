//! RunView — live execution output (`besogne run` or standalone binary).
//!
//! Sub-states (same view, different branches):
//!   first-run:    seal probes + exec DAG + verification
//!   skip:         everything cached → SkipBanner (single line, early return)
//!   seal-changed: some probes differ → ChangedProbes + re-execute
//!   failure:      probe or command fails → DiagBlock + FooterLine(fail)
//!
//! Sections: Header? → Build? → Seal → Exec → Diagnostic? → Footer + MetricLine
//!
//! Context flags:
//!   BESOGNE_RUN_MODE: hides Header and Build (compiler already showed them)
//!   --verbose: all L3 components visible (build nodes, env, per-cmd metrics, process tree)
//!
//! L3 components used:
//!   sections::phase_banner        — phase headers
//!   sections::footer_line         — final status
//!   sections::skip_banner         — nothing to do
//!   sections::diag_block          — errors/warnings
//!   items::probe_item             — seal probe results
//!   items::command_block           — command execution
//!   items::progress_step          — build steps (verbose)
//!   items::verify_result          — idempotency check
//!   items::changed_probes         — seal invalidation
//!   atoms::status_badge           — status labels
//!   atoms::node_badge             — node type badges
//!   atoms::binary_ref             — resolved binary paths
//!   atoms::exit_code              — colored exit codes
//!   telemetry::metric_line        — per-command + aggregate metrics

/// Configuration for the run view.
pub struct RunViewConfig {
    /// Whether header and build section should be shown.
    /// false when BESOGNE_RUN_MODE (compiler already showed build).
    pub show_header: bool,
    /// Show all L3 components (build nodes, env, per-cmd metrics, process tree).
    pub verbose: bool,
}

impl Default for RunViewConfig {
    fn default() -> Self {
        Self {
            show_header: true,
            verbose: false,
        }
    }
}

// ── Sub-state renderers ─────────────────────────────────────────────────

/// Render the skip state: everything cached, nothing to do.
pub fn render_skip(total_nodes: usize, ran_at: &str, duration_ms: u64) {
    use crate::output::style::l3;
    eprintln!("{}", l3::sections::skip_banner::render(total_nodes, ran_at, duration_ms));
}

// TODO: render_seal() — seal phase with probes (ProbeItem* + ChangedProbes?)
// TODO: render_exec() — exec phase with commands (CommandBlock* + VerifyResult*)
// TODO: render_diagnostic() — collect warnings/errors into DiagBlock
// TODO: render_footer() — FooterLine + aggregate MetricLine

// The full run view is orchestrated by runtime/mod.rs which calls
// OutputRenderer trait methods. Migration path:
//   1. RunView functions replace HumanRenderer methods one by one
//   2. RunView becomes the single entry point for human output
//   3. OutputRenderer trait is retired for human format
