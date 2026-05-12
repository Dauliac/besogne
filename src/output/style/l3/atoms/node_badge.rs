//! NodeBadge: "bin" in node type color.
//! Axes: identity domain → node color + badge text.

use crate::output::style::styled;

/// Format a node type badge in its color.
pub fn render(badge_text: &str, node_color: &str) -> String {
    styled(node_color, badge_text)
}
