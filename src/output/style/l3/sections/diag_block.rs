//! DiagBlock: rustc-style error/warning diagnostics.
//! Axes: diagnostic x outcome::{FAIL|WARN} x weight::L1.
//!
//! Produces output like:
//!   error: binary 'foo' not found
//!     --> manifest [nodes.foo]
//!      |
//!      |  [nodes.foo]
//!      |  type = "binary"
//!      |
//!      = note: something
//!      = hint: do something about it

use crate::output::style::{diagnostic, palette::RESET};

pub fn error(msg: &str) -> String {
    format!("{}error{RESET}: {}", diagnostic::ERROR, msg)
}

pub fn warning(msg: &str) -> String {
    format!("{}warning{RESET}: {}", diagnostic::WARN, msg)
}

pub fn hint(msg: &str) -> String {
    format!("{}hint{RESET}: {msg}", diagnostic::HINT)
}

/// Builder for rustc-style diagnostic blocks with gutter.
pub struct DiagBuilder {
    lines: Vec<String>,
}

impl DiagBuilder {
    pub fn new() -> Self {
        Self { lines: Vec::new() }
    }

    /// `  --> location`
    pub fn location(mut self, loc: &str) -> Self {
        self.lines.push(format!("  {}  -->{RESET} {loc}", diagnostic::GUTTER));
        self
    }

    /// `   |`
    pub fn blank(mut self) -> Self {
        self.lines.push(format!("  {}   |{RESET}", diagnostic::GUTTER));
        self
    }

    /// `   |  content`
    pub fn code(mut self, content: &str) -> Self {
        self.lines.push(format!("  {}   |{RESET}  {}{content}{RESET}",
            diagnostic::GUTTER, diagnostic::CODE));
        self
    }

    /// `   = message`
    pub fn note(mut self, msg: &str) -> Self {
        self.lines.push(format!("  {}   ={RESET} {msg}", diagnostic::GUTTER));
        self
    }

    /// `   = hint: message`
    pub fn hint(mut self, msg: &str) -> Self {
        self.lines.push(format!("  {}   ={RESET} {}", diagnostic::GUTTER, hint(msg)));
        self
    }

    pub fn build(self) -> String {
        self.lines.join("\n")
    }
}
