//! Text tokens: messages and hints (diagnostic + structure domains).

pub const CACHE_DISABLED: &str = "cache disabled until manifest is updated";
pub const UNDECLARED_DEPS: &str = "warning: undeclared dependencies detected at runtime";
pub const STATUS_HINT: &str = "use --status to show last run";
pub const PROCESS_TREE: &str = "process tree";
pub const LAST_RUN: &str = "last run:";
pub const NODES_CACHED: &str = "nodes cached";
pub const SIDE_EFFECTS: &str = "(side_effects)";
pub const SECRET: &str = "(secret)";
pub const RAN: &str = "ran";
pub const IDEMPOTENT: &str = "idempotent";
pub const NOT_IDEMPOTENT: &str = "NOT idempotent";
pub const VERIFY_RUN2: &str = "verifying idempotency (re-run)";
pub const VERIFY_MISMATCH_EXIT: &str = "exit code differs";
pub const VERIFY_MISMATCH_STDOUT: &str = "stdout differs";
pub const VERIFY_MISMATCH_STDERR: &str = "stderr differs";
