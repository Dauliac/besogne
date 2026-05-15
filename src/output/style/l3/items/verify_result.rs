//! VerifyResult: ✓ idempotent  /  ✗ NOT idempotent \[diff\]
//! Axes: diagnostic x outcome x phase::exec.

use crate::output::style::{styled, diagnostic, message, icon};

pub fn ok(name: &str) -> String {
    format!("    {} {} {}",
        styled(diagnostic::IDEMPOTENT, icon::OK),
        styled(diagnostic::IDEMPOTENT, name),
        styled(diagnostic::IDEMPOTENT, message::IDEMPOTENT))
}

pub fn fail(name: &str) -> String {
    format!("    {} {} {}",
        styled(diagnostic::NOT_IDEMPOTENT, icon::FAIL),
        styled(diagnostic::NOT_IDEMPOTENT, name),
        styled(diagnostic::NOT_IDEMPOTENT, message::NOT_IDEMPOTENT))
}
