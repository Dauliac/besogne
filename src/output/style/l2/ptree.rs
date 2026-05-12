//! Domain: Telemetry — process tree (sub-subsystem).
//! All L3 except: root process = L2, failed child = escalated to RED.

use crate::output::style::palette;

pub const CONNECTOR: &str = palette::DIM;       // ├── └── │
pub const ROOT: &str = palette::WHITE;          // root process name — L2 brightness
pub const CHILD: &str = palette::DIM;           // child process names — L3
pub const SUBSHELL: &str = palette::DIM;        // "(subshell)" annotation
pub const EXIT_OK: &str = palette::DIM_GREEN;   // [0] — expected, boring
pub const EXIT_FAIL: &str = palette::RED;       // [1] — escalated!
