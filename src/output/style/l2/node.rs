//! Domain: Identity — node type colors.
//! Each node type has a hue loosely mapping to its typical phase.

use crate::output::style::palette;

pub const BINARY: &str = palette::CYAN;       // build/exec adjacent
pub const FILE: &str = palette::MAGENTA;      // build/seal adjacent
pub const ENV: &str = palette::YELLOW;        // seal — warm, environment
pub const COMMAND: &str = palette::BOLD_WHITE; // exec — neutral, main actor
pub const SERVICE: &str = palette::GREEN;     // exec — alive, running
pub const PLATFORM: &str = palette::CYAN;     // build — system info
pub const DNS: &str = palette::GREEN;         // exec — network, alive
pub const METRIC: &str = palette::YELLOW;     // exec — measurement
pub const SOURCE: &str = palette::CYAN;       // seal — data source
pub const STD: &str = palette::GREEN;         // exec — stream
