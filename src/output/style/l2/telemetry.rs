//! Domain: Telemetry (scoped subsystem).
//! Color = category, not outcome. Normal weight for readability.
//! Icons do differentiation. Colors are base hues (not dim).

use crate::output::style::palette;

// Category colors (normal weight — readable on dark terminals)
pub const TIME: &str = palette::CYAN;           // exec-adjacent (action duration)
pub const CPU: &str = palette::YELLOW;          // warm = computation
pub const MEMORY: &str = palette::MAGENTA;      // build-adjacent (resource weight)
pub const DISK: &str = palette::BLUE;           // seal-adjacent (I/O, persistence)
pub const NETWORK: &str = palette::GREEN;       // connectivity
pub const PROCESS: &str = palette::WHITE;       // structural

// Anomaly escalation: breaks out of L3 → L2
pub const ANOMALY: &str = palette::YELLOW;

// Icons
pub const TIME_ICON: &str = "\u{23f1}\u{fe0f}"; // ⏱️
pub const CPU_ICON: &str = "\u{26a1}";           // ⚡
pub const MEMORY_ICON: &str = "\u{1f9e0}";       // 🧠
pub const DISK_ICON: &str = "\u{1f4be}";         // 💾
pub const NETWORK_ICON: &str = "\u{1f310}";      // 🌐
pub const PROCESS_ICON: &str = "\u{1f500}";      // 🔀
