//! Axis 3: Outcome — WHAT happened.
//! Traffic light: ok → green, warn → yellow, fail → red.

use crate::output::style::palette;

// ── Enum ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Outcome {
    Ok,
    Warn,
    Fail,
}

impl Outcome {
    /// Base color at L2 weight.
    pub const fn color(self) -> &'static str {
        match self {
            Outcome::Ok => palette::GREEN,
            Outcome::Warn => palette::YELLOW,
            Outcome::Fail => palette::RED,
        }
    }

    /// L1 bold variant (escalated).
    pub const fn color_bold(self) -> &'static str {
        match self {
            Outcome::Ok => palette::BOLD_GREEN,
            Outcome::Warn => palette::BOLD_YELLOW,
            Outcome::Fail => palette::BOLD_RED,
        }
    }

    /// L3 dim variant.
    pub const fn color_dim(self) -> &'static str {
        match self {
            Outcome::Ok => palette::DIM_GREEN,
            Outcome::Warn => palette::DIM_YELLOW,
            Outcome::Fail => palette::DIM_RED,
        }
    }
}

// ── Const tokens ─────────────────────────────────────────────────────────

pub const OK: &str = palette::GREEN;
pub const WARN: &str = palette::YELLOW;
pub const FAIL: &str = palette::RED;

pub const OK_BOLD: &str = palette::BOLD_GREEN;
pub const WARN_BOLD: &str = palette::BOLD_YELLOW;
pub const FAIL_BOLD: &str = palette::BOLD_RED;

pub const OK_DIM: &str = palette::DIM_GREEN;
pub const WARN_DIM: &str = palette::DIM_YELLOW;
pub const FAIL_DIM: &str = palette::DIM_RED;
