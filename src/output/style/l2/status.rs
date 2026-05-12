//! Cross-axis composite: phase x outcome x temporality → status color.
//! The status color tells you BOTH where (phase) and what (outcome).

use crate::output::style::palette;

// build x ok x static → phase hue
pub const PINNED: &str = palette::MAGENTA;

// seal x ok x cached → phase hue
pub const SEALED: &str = palette::BLUE;

// seal x ok x live → outcome hue
pub const PROBED: &str = palette::GREEN;

// exec x ok x live → outcome hue
pub const OK: &str = palette::GREEN;

// exec x ok x cached → temporality dims
pub const CACHED: &str = palette::DIM;

// varies x fail x live → outcome hue
pub const FAILED: &str = palette::RED;

// seal x warn x live → outcome hue
pub const CHANGED: &str = palette::YELLOW;

// global x warn x cached → outcome hue
pub const SKIP: &str = palette::YELLOW;

// varies x none → neutral
pub const PENDING: &str = palette::BLUE;

// exec x none x live → meta-action
pub const VERIFY: &str = palette::DIM;

// backward-compat aliases
pub const FRESH: &str = OK;
pub const INVALIDATED: &str = CHANGED;
