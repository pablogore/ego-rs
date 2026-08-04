//! Compatibility re-export.
//!
//! `Clock`/`SystemClock` moved to [`crate::time::clock`], generalized for
//! use beyond authentication. This module keeps the original path working,
//! unchanged in behavior, for every existing call site (JWT expiry/`nbf`
//! checks and any other consumer of `ego_domain::auth::clock::Clock` or the
//! crate-level `ego_domain::Clock` re-export). No `#[deprecated]` attribute
//! is used — this workspace treats warnings as errors, so a deprecation
//! notice here would fail the build at every existing use site rather than
//! merely warn.

pub use crate::time::clock::{Clock, SystemClock};
