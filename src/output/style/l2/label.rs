//! Text tokens: status labels (state domain).

pub const PINNED: &str = "pinned";
pub const SEALED: &str = "sealed";
pub const PROBED: &str = "probed";
pub const OK: &str = "ok";
pub const CACHED: &str = "cached";
pub const FAILED: &str = "fail";
pub const CHANGED: &str = "changed";
pub const SKIP: &str = "nothing to do";
pub const DONE: &str = "done";
pub const PENDING: &str = "pending";
pub const PASSED: &str = "passed";
pub const SKIPPED: &str = "skipped";
pub const EXEC: &str = "exec";

// backward-compat aliases
pub const FRESH: &str = OK;
pub const NOTHING_TO_DO: &str = SKIP;
