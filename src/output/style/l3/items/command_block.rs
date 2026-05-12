//! CommandBlock: ▹ name: exec args...  [output]  status
//! Axes: identity x output x state x phase::exec x temporality → weight.
//!
//! Sub-parts:
//!   header:       [L1] name + [L2] args
//!   stdout_live:  [L2] (happening now)
//!   stdout_cache: [L3] (from before)
//!   status_ok:    [L2] outcome::OK
//!   status_fail:  [L1] outcome::FAIL (escalated)
//!
//! TODO: implement when renderers are refactored to use L3 components.
