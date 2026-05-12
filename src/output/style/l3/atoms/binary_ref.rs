//! BinaryRef: echo → /usr/bin/echo v9.4
//! Axes: identity x phase::build x temporality::static.
//!   name:    [L2] (readable)
//!   arrow:   [L3] dim
//!   path:    [L3] dim
//!   version: [L3] dim

use crate::output::style::{dim, weight, palette::RESET};

pub fn render(name: &str, path: &str, version: Option<&str>) -> String {
    let ver = version
        .map(|v| format!(" {}v{v}{RESET}", weight::L3))
        .unwrap_or_default();
    format!("{name} {}→ {path}{RESET}{ver}", weight::L3)
}
