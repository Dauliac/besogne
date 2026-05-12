//! Axis 5: Weight — HOW important.
//! L1 bold (demands attention), L2 normal, L3 dim (recedes).

use crate::output::style::palette;

// ── Enum ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Weight {
    /// Structural landmarks, errors, final verdict.
    L1,
    /// Active content, current action.
    L2,
    /// Background info, cached, context, metadata.
    L3,
}

impl Weight {
    /// ANSI modifier string. L2 returns empty (no modifier).
    pub const fn modifier(self) -> &'static str {
        match self {
            Weight::L1 => palette::BOLD,
            Weight::L2 => "",
            Weight::L3 => palette::DIM,
        }
    }

    /// Escalate toward L1. Returns the more prominent of self and other.
    pub const fn escalate(self, other: Weight) -> Weight {
        // L1 < L2 < L3 in the Ord, so min = most prominent
        if (self as u8) < (other as u8) { self } else { other }
    }
}

// ── Const tokens ─────────────────────────────────────────────────────────

pub const L1: &str = palette::BOLD;
pub const L2: &str = "";
pub const L3: &str = palette::DIM;
