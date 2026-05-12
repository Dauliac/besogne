//! Axis 4: Temporality — WHEN did it happen.
//! Temporality has no colors — it modifies default weight.
//!   live   → L2 (happening now, readable)
//!   cached → L3 (old news, dim)
//!   static → L3 (immutable context, dim)

use super::weight::Weight;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Temporality {
    /// Happening right now in this execution.
    Live,
    /// From a previous run, replayed.
    Cached,
    /// Frozen at build time, immutable.
    Static,
}

impl Temporality {
    /// Default weight for this temporality.
    pub const fn default_weight(self) -> Weight {
        match self {
            Temporality::Live => Weight::L2,
            Temporality::Cached => Weight::L3,
            Temporality::Static => Weight::L3,
        }
    }
}
