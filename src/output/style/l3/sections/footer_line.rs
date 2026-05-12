//! FooterLine: done 0.088s  /  fail exit 1  0.234s
//! Axes: structure x outcome x weight::L1.

use crate::output::style::{styled, dim, outcome, label};

pub fn render(exit_code: i32, wall_ms: u64) -> String {
    let timing = dim(&format!("{:.3}s", wall_ms as f64 / 1000.0));
    if exit_code == 0 {
        format!("{} {timing}", styled(outcome::OK_BOLD, label::DONE))
    } else {
        format!("{} {} {timing}",
            styled(outcome::FAIL_BOLD, label::FAILED),
            styled(outcome::FAIL_BOLD, &format!("exit {exit_code}")))
    }
}
