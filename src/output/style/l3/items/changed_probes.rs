//! ChangedProbes: △ 1 probe changed → re-executing
//! Axes: diagnostic x outcome::warn x state.

use crate::output::style::{styled, status, icon};

pub fn render(changed: &[String]) -> String {
    format!("\n  {} {} changed → re-executing",
        styled(status::CHANGED, icon::CHANGED),
        changed.join(", "))
}
