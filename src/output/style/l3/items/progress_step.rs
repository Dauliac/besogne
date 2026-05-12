//! ProgressStep: • parsed besogne.toml (12ms)
//! Axes: structure x weight::L3 (all dim — mechanical step).

use crate::output::style::{dim, layout};

pub fn render(text: &str) -> String {
    format!("  {} {}", dim(layout::STEP_BULLET), dim(text))
}
