//! L3 — Component tokens.
//! Pure rendering functions: take primitives, return styled strings.
//! Organized by visual role:
//!
//!   atoms/      smallest styled fragments (L2 → String)
//!   items/      one row or card inside a section (atoms → String)
//!   sections/   structural frame around items (items + atoms → String)
//!   telemetry/  metrics and process trees (own visual subsystem)
//!
//! Dependency rule: sections → items → atoms, telemetry → atoms.

pub mod atoms;
pub mod items;
pub mod sections;
pub mod telemetry;
