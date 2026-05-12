//! Design tokens for terminal output.
//!
//! Two-layer system:
//!   Layer 1 — Palette: raw ANSI codes, no semantics
//!   Layer 2 — Tokens: semantic names mapped to palette entries
//!
//! All rendering code uses tokens, never raw ANSI codes.
//! Changing a color = change one line in this file.

// ── Layer 1: Palette ─────────────────────────────────────────────────
// Raw ANSI escape codes. Named by visual appearance, not meaning.

pub mod palette {
    pub const RESET: &str = "\x1b[0m";
    pub const BOLD: &str = "\x1b[1m";
    pub const DIM: &str = "\x1b[2m";

    pub const RED: &str = "\x1b[31m";
    pub const GREEN: &str = "\x1b[32m";
    pub const YELLOW: &str = "\x1b[33m";
    pub const BLUE: &str = "\x1b[34m";
    pub const MAGENTA: &str = "\x1b[35m";
    pub const CYAN: &str = "\x1b[36m";
    pub const WHITE: &str = "\x1b[37m";

    pub const BOLD_RED: &str = "\x1b[1;31m";
    pub const BOLD_GREEN: &str = "\x1b[1;32m";
    pub const BOLD_YELLOW: &str = "\x1b[1;33m";
    pub const BOLD_BLUE: &str = "\x1b[1;34m";
    pub const BOLD_WHITE: &str = "\x1b[1;37m";

    pub const DIM_BLUE: &str = "\x1b[2;34m";
    pub const DIM_CYAN: &str = "\x1b[2;36m";
}

// ── Layer 2: Tokens ──────────────────────────────────────────────────
// Semantic names. Every color decision in the codebase points here.

/// Node lifecycle status
pub mod status {
    use super::palette::*;
    pub const PINNED: &str = GREEN;      // build-time frozen — validated
    pub const SEALED: &str = GREEN;      // reused from cache — validated
    pub const PROBED: &str = GREEN;      // just probed/verified — validated
    pub const FAILED: &str = RED;        // probe or command failed
    pub const PENDING: &str = BLUE;      // not yet evaluated — to run
    pub const SKIP: &str = YELLOW;       // skipped (nothing to do)
    pub const INVALIDATED: &str = YELLOW; // changed, triggers re-execution
    // General-purpose aliases (for commands, summaries, non-probe contexts)
    pub const FRESH: &str = GREEN;       // command succeeded — validated
    pub const CACHED: &str = GREEN;      // command replayed from cache — validated
}

/// Node type badges
pub mod node {
    use super::palette::*;
    pub const BINARY: &str = CYAN;
    pub const FILE: &str = MAGENTA;
    pub const ENV: &str = YELLOW;
    pub const COMMAND: &str = BOLD_WHITE;
    pub const SERVICE: &str = GREEN;

    pub const PLATFORM: &str = CYAN;
    pub const DNS: &str = GREEN;
    pub const METRIC: &str = YELLOW;
    pub const SOURCE: &str = CYAN;
    pub const STD: &str = GREEN;
}

/// Metric categories (emojis + colors)
pub mod metric {
    use super::palette::*;
    pub const TIME: &str = CYAN;
    pub const TIME_ICON: &str = "\u{23f1}\u{fe0f}";  // ⏱️
    pub const CPU: &str = YELLOW;
    pub const CPU_ICON: &str = "\u{26a1}";            // ⚡
    pub const MEMORY: &str = MAGENTA;
    pub const MEMORY_ICON: &str = "\u{1f9e0}";        // 🧠
    pub const DISK: &str = BLUE;
    pub const DISK_ICON: &str = "\u{1f4be}";          // 💾
    pub const NETWORK: &str = GREEN;
    pub const NETWORK_ICON: &str = "\u{1f310}";       // 🌐
    pub const PROCESS: &str = RED;
    pub const PROCESS_ICON: &str = "\u{1f500}";       // 🔀
}

/// Phase labels
pub mod phase {
    use super::palette::*;
    pub const BUILD: &str = BOLD_BLUE;
    pub const SEAL: &str = BOLD_YELLOW;
    pub const EXEC: &str = BOLD_GREEN;
}

/// Exit code coloring
pub mod exit {
    use super::palette::*;
    pub const OK: &str = GREEN;
    pub const FAIL: &str = RED;
}

/// Text emphasis
pub mod emphasis {
    use super::palette::*;
    pub const PRIMARY: &str = BOLD;
    pub const SECONDARY: &str = DIM;
    pub const ACCENT: &str = CYAN;
}

/// Structural elements (errors, warnings, tree)
pub mod structure {
    use super::palette::*;
    pub const ERROR: &str = BOLD_RED;
    pub const WARNING: &str = BOLD_YELLOW;
    pub const HINT: &str = YELLOW;
    pub const GUTTER: &str = BOLD_BLUE;    // rustc-style `-->` and `|`
    pub const TREE_HEADER: &str = DIM_CYAN;
    pub const BACKREF: &str = DIM;
}

// ── Layer 3: Text tokens ─────────────────────────────────────────────
// Canonical wording. Every user-facing string points here.

/// Status labels (probe/command outcomes)
pub mod label {
    pub const PINNED: &str = "pinned";
    pub const SEALED: &str = "sealed";
    pub const PROBED: &str = "probed";
    pub const FAILED: &str = "fail";
    pub const PENDING: &str = "pending";
    pub const DONE: &str = "done";
    pub const NOTHING_TO_DO: &str = "nothing to do";
    pub const PASSED: &str = "passed";
    pub const SKIPPED: &str = "skipped";
    pub const EXEC: &str = "exec";
    // General-purpose aliases (for commands, summaries, non-probe contexts)
    pub const FRESH: &str = "ok";
    pub const CACHED: &str = "cached";
}

/// Node type short names (for badges)
pub mod badge {
    pub const BINARY: &str = " bin ";
    pub const FILE: &str = "file";
    pub const ENV: &str = " env ";
    pub const COMMAND: &str = " cmd ";
    pub const SERVICE: &str = " svc ";

    pub const PLATFORM: &str = "plat";
    pub const DNS: &str = " dns ";
    pub const METRIC: &str = " met ";
    pub const SOURCE: &str = " src ";
    pub const STD: &str = " std ";
}

/// Phase names
pub mod phase_label {
    pub const BUILD: &str = "build";
    pub const SEAL: &str = "seal";
    pub const EXEC: &str = "exec";
}

/// Metric labels
pub mod metric_label {
    pub const TIME: &str = "time";
    pub const CPU: &str = "cpu";
    pub const MEMORY: &str = "memory";
    pub const READ: &str = "read";
    pub const WRITE: &str = "write";
    pub const DOWNLOAD: &str = "download";
    pub const UPLOAD: &str = "upload";
    pub const PROCESSES: &str = "processes";
    pub const USER: &str = "user";
    pub const KERNEL: &str = "kernel";
    pub const CORES: &str = "cores";
}

/// Messages and hints
pub mod message {
    pub const CACHE_DISABLED: &str = "Cache disabled until manifest is updated.";
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
}

/// Verification status
pub mod verify {
    use super::palette::*;
    pub const IDEMPOTENT: &str = GREEN;
    pub const NOT_IDEMPOTENT: &str = RED;
    pub const VERIFYING: &str = DIM;
}

// ── Helpers ──────────────────────────────────────────────────────────

use palette::RESET;

/// Wrap text in a token color, auto-resetting.
pub fn styled(token: &str, text: &str) -> String {
    format!("{token}{text}{RESET}")
}

/// Wrap text in dim.
pub fn dim(text: &str) -> String {
    format!("{}{text}{RESET}", palette::DIM)
}

/// Wrap text in bold.
pub fn bold(text: &str) -> String {
    format!("{}{text}{RESET}", palette::BOLD)
}

/// Format an exit code with success/failure coloring.
pub fn exit_code(code: i32) -> String {
    if code == 0 {
        styled(exit::OK, "0")
    } else {
        styled(exit::FAIL, &code.to_string())
    }
}

/// Format a status badge: ` sealed `, ` cached `, etc.
pub fn status_badge(label: &str, token: &str) -> String {
    styled(token, &format!(" {label:^7}"))
}

// ── Diagnostic formatters (rustc-style) ─────────────────────────────
// Produces output like:
//   error: binary 'foo' not found
//     --> manifest [nodes.foo]
//      |
//      |  [nodes.foo]
//      |  type = "binary"
//      |
//      = some detail
//      = hint: do something about it

/// Format an `error:` prefix.
pub fn diag_error(msg: &str) -> String {
    format!("{}{RESET}: {}", structure::ERROR, msg)
}

/// Format a `warning:` prefix.
pub fn diag_warning(msg: &str) -> String {
    format!("{}{RESET}: {}", structure::WARNING, msg)
}

/// Format a `hint:` line.
pub fn diag_hint(msg: &str) -> String {
    format!("{}hint{RESET}: {msg}", structure::HINT)
}

/// Format gutter lines for rustc-style diagnostics.
pub struct DiagBuilder {
    lines: Vec<String>,
}

impl DiagBuilder {
    pub fn new() -> Self {
        Self { lines: Vec::new() }
    }

    /// Add the `  --> location` line.
    pub fn location(mut self, loc: &str) -> Self {
        self.lines.push(format!("  {}  -->{RESET} {loc}", structure::GUTTER));
        self
    }

    /// Add a blank gutter line `   |`.
    pub fn blank(mut self) -> Self {
        self.lines.push(format!("  {}   |{RESET}", structure::GUTTER));
        self
    }

    /// Add a code line `   |  content`.
    pub fn code(mut self, content: &str) -> Self {
        self.lines.push(format!("  {}   |{RESET}  {content}", structure::GUTTER));
        self
    }

    /// Add a note line `   = message`.
    pub fn note(mut self, msg: &str) -> Self {
        self.lines.push(format!("  {}   ={RESET} {msg}", structure::GUTTER));
        self
    }

    /// Add a hint line `   = hint: message`.
    pub fn hint(mut self, msg: &str) -> Self {
        self.lines.push(format!("  {}   ={RESET} {}", structure::GUTTER, diag_hint(msg)));
        self
    }

    /// Build the final diagnostic block.
    pub fn build(self) -> String {
        self.lines.join("\n")
    }
}

/// Format a complete error diagnostic.
pub fn error_diag(msg: &str) -> String {
    diag_error(msg)
}

/// Format a complete warning diagnostic.
pub fn warning_diag(msg: &str) -> String {
    diag_warning(msg)
}
