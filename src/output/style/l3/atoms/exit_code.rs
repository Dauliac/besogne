//! ExitCode: colored exit code.
//! exit 0 = L3 dim green (boring). exit N = L1 bold red (escalated).
//! Axes: output x outcome → escalation rule.

use crate::output::style::{styled, outcome};

pub fn render(code: i32) -> String {
    if code == 0 {
        styled(outcome::OK_DIM, "0")
    } else {
        styled(outcome::FAIL_BOLD, &code.to_string())
    }
}
