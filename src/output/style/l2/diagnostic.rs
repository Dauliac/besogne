//! Domain: Diagnostic — errors, warnings, hints, verification.
//! Uses outcome colors at L1 weight.

use crate::output::style::palette;

pub const ERROR: &str = palette::BOLD_RED;      // L1 always
pub const WARN: &str = palette::BOLD_YELLOW;    // L1 always
pub const HINT: &str = palette::YELLOW;          // "hint:" keyword color
pub const GUTTER: &str = palette::BOLD_BLUE;    // rustc-style | and -->
pub const CODE: &str = palette::WHITE;          // code lines in diagnostic

// Verification
pub const IDEMPOTENT: &str = palette::GREEN;        // outcome::OK
pub const NOT_IDEMPOTENT: &str = palette::RED;      // outcome::FAIL
pub const VERIFYING: &str = palette::DIM;           // weight::L3
