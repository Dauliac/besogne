//! Domain: Telemetry (scoped subsystem).
//! Always L3 globally — background info. Color = category, not outcome.
//! Icons do differentiation. Colors are DIM variants of phase-adjacent hues.

use crate::output::style::palette;

// Category colors (dim — never compete with pipeline output)
pub const TIME: &str = palette::DIM_CYAN;       // exec-adjacent (action duration)
pub const CPU: &str = palette::DIM_YELLOW;      // warm = computation
pub const MEMORY: &str = palette::DIM_MAGENTA;  // build-adjacent (resource weight)
pub const DISK: &str = palette::DIM_BLUE;       // seal-adjacent (I/O, persistence)
pub const NETWORK: &str = palette::DIM_GREEN;   // connectivity
pub const PROCESS: &str = palette::DIM;         // structural, neutral

// Anomaly escalation: breaks out of L3 → L2
pub const ANOMALY: &str = palette::YELLOW;

// Icons
pub const TIME_ICON: &str = "\u{23f1}\u{fe0f}"; // ⏱️
pub const CPU_ICON: &str = "\u{26a1}";           // ⚡
pub const MEMORY_ICON: &str = "\u{1f9e0}";       // 🧠
pub const DISK_ICON: &str = "\u{1f4be}";         // 💾
pub const NETWORK_ICON: &str = "\u{1f310}";      // 🌐
pub const PROCESS_ICON: &str = "\u{1f500}";      // 🔀
