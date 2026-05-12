//! L1 — Raw ANSI escape codes.
//! Named by visual appearance only. No semantics attached here.
//! Every color decision in the codebase traces back through L2 tokens to these.

pub const RESET: &str = "\x1b[0m";
pub const BOLD: &str = "\x1b[1m";
pub const DIM: &str = "\x1b[2m";

// Base hues
pub const RED: &str = "\x1b[31m";
pub const GREEN: &str = "\x1b[32m";
pub const YELLOW: &str = "\x1b[33m";
pub const BLUE: &str = "\x1b[34m";
pub const MAGENTA: &str = "\x1b[35m";
pub const CYAN: &str = "\x1b[36m";
pub const WHITE: &str = "\x1b[37m";

// Bold (L1 weight + hue)
pub const BOLD_RED: &str = "\x1b[1;31m";
pub const BOLD_GREEN: &str = "\x1b[1;32m";
pub const BOLD_YELLOW: &str = "\x1b[1;33m";
pub const BOLD_BLUE: &str = "\x1b[1;34m";
pub const BOLD_MAGENTA: &str = "\x1b[1;35m";
pub const BOLD_CYAN: &str = "\x1b[1;36m";
pub const BOLD_WHITE: &str = "\x1b[1;37m";

// Dim (L3 weight + hue)
pub const DIM_RED: &str = "\x1b[2;31m";
pub const DIM_GREEN: &str = "\x1b[2;32m";
pub const DIM_YELLOW: &str = "\x1b[2;33m";
pub const DIM_BLUE: &str = "\x1b[2;34m";
pub const DIM_MAGENTA: &str = "\x1b[2;35m";
pub const DIM_CYAN: &str = "\x1b[2;36m";
pub const DIM_WHITE: &str = "\x1b[2;37m";
