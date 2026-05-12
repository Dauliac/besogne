//! CheckView — `besogne check` output.
//!
//! Shows validation result per manifest.
//!
//! L3 components used:
//!   sections::diag_block (on validation error)

use crate::output::style::{styled, outcome};
use crate::output::style::l3;

pub struct CheckResult<'a> {
    pub path: &'a str,
    pub error: Option<&'a str>,
}

pub fn render(results: &[CheckResult]) {
    for r in results {
        if let Some(err) = r.error {
            eprintln!("{}", l3::sections::diag_block::error(
                &format!("{}: {err}", r.path)));
        } else {
            eprintln!("  {} {} is valid",
                styled(outcome::OK, "\u{2713}"),
                r.path);
        }
    }
}
