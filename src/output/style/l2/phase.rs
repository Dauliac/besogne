//! Axis 2: Phase — WHERE in the pipeline.
//! build → magenta, seal → blue, exec → cyan.

use crate::output::style::palette;

// ── Enum (for type-safe function signatures + match) ─────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Phase {
    Build,
    Seal,
    Exec,
}

impl Phase {
    /// L1 bold color for phase headers.
    pub const fn color(self) -> &'static str {
        match self {
            Phase::Build => palette::BOLD_MAGENTA,
            Phase::Seal => palette::BOLD_BLUE,
            Phase::Exec => palette::BOLD_CYAN,
        }
    }

    /// L3 dim color for phase-colored content in background.
    pub const fn color_dim(self) -> &'static str {
        match self {
            Phase::Build => palette::DIM_MAGENTA,
            Phase::Seal => palette::DIM_BLUE,
            Phase::Exec => palette::DIM_CYAN,
        }
    }

    /// Canonical phase name.
    pub const fn label(self) -> &'static str {
        match self {
            Phase::Build => "build",
            Phase::Seal => "seal",
            Phase::Exec => "exec",
        }
    }
}

// ── Const tokens (for zero-cost format! interpolation) ───────────────────

pub const BUILD: &str = palette::BOLD_MAGENTA;
pub const SEAL: &str = palette::BOLD_BLUE;
pub const EXEC: &str = palette::BOLD_CYAN;

pub const BUILD_DIM: &str = palette::DIM_MAGENTA;
pub const SEAL_DIM: &str = palette::DIM_BLUE;
pub const EXEC_DIM: &str = palette::DIM_CYAN;
