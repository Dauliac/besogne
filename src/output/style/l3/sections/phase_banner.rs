//! PhaseBanner: ▸ build (44 nodes)
//! Axes: structure x phase x weight::L1 (name) + weight::L3 (detail).

use crate::output::style::{styled, dim, layout, phase::Phase};

pub fn render(p: Phase, count: usize, detail: Option<&str>) -> String {
    let extra = detail
        .map(|d| format!(" {}", dim(d)))
        .unwrap_or_default();
    format!("{} {}{}",
        styled(p.color(), layout::PHASE_BULLET),
        styled(p.color(), &format!("{} ({count} nodes)", p.label())),
        extra)
}
