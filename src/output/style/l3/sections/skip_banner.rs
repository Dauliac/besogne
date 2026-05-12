//! SkipBanner: nothing to do (52 nodes cached, ran 22s ago, use --status)
//! Axes: state x outcome::warn x temporality::cached.

use crate::output::style::{styled, dim, bold, status, label, message};

pub fn render(total_nodes: usize, ran_at: &str, duration_ms: u64) -> String {
    format!("{} {}",
        styled(status::SKIP, label::SKIP),
        dim(&format!("({total_nodes} {}, ran {ran_at}, {:.3}s, {})",
            message::NODES_CACHED, duration_ms as f64 / 1000.0,
            message::STATUS_HINT)))
}
