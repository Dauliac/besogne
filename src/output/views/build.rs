//! BuildView — `besogne build` output.
//!
//! Two modes:
//!   Single manifest: PhaseBanner → ProgressStep* → FooterLine
//!   Multi manifest:  PhaseBanner → BuildResult* → FooterLine
//!
//! L3 components used:
//!   sections::phase_banner
//!   sections::footer_line
//!   items::progress_step
//!   telemetry::metric_line (footer aggregate)

use crate::output::style::l3;
use crate::output::style::phase::Phase;

/// Render a single-manifest build with step-by-step progress.
pub fn render_single(steps: &[&str], total_ms: u64) {
    eprintln!("{}", l3::sections::phase_banner::render(Phase::Build, 0, None));
    for step in steps {
        eprintln!("{}", l3::items::progress_step::render(step));
    }
    eprintln!();
    eprintln!("{}", l3::sections::footer_line::render(0, total_ms));
}

/// Result of building one manifest in a multi-build.
pub struct ManifestResult<'a> {
    pub name: &'a str,
    pub node_count: usize,
    pub duration_ms: u64,
    pub error: Option<&'a str>,
}

/// Render a multi-manifest parallel build.
pub fn render_multi(results: &[ManifestResult], total_ms: u64) {
    use crate::output::style::{styled, dim};
    use crate::output::style::{outcome, status};

    let count = results.len();
    eprintln!("{}", l3::sections::phase_banner::render(
        Phase::Build, count, Some(&format!("{count} manifests"))));

    let mut ok_count = 0;
    let mut fail_count = 0;
    for r in results {
        if let Some(err) = r.error {
            eprintln!("  {} {}: {err}",
                styled(outcome::FAIL, "\u{2717}"),  // ✗
                r.name);
            fail_count += 1;
        } else {
            eprintln!("  {} {} {}",
                styled(outcome::OK, "\u{2713}"),  // ✓
                r.name,
                dim(&format!("({} nodes in {}ms)", r.node_count, r.duration_ms)));
            ok_count += 1;
        }
    }

    eprintln!();
    let exit = if fail_count > 0 { 1 } else { 0 };
    let summary = format!("({ok_count} ok, {fail_count} fail)");
    eprintln!("{} {}",
        l3::sections::footer_line::render(exit, total_ms),
        dim(&summary));
}
