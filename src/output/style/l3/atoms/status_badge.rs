//! StatusBadge: " pinned " in status color.
//! Axes: state x phase x outcome → status color + label text.

use crate::output::style::{styled, palette::RESET};

/// Format a status badge: ` sealed `, ` cached `, etc.
pub fn render(label: &str, token: &str) -> String {
    styled(token, &format!(" {label:^7}"))
}
