//! Scheduling policies for execution dispatch.
//!
//! This module is a documentation-only contract. The actual scheduling
//! mechanism is left to backend implementations.
//!
//! # Contract
//!
//! - Messages for a single execution are processed sequentially.
//! - The runtime must provide a default scheduling strategy suitable
//!   for general-purpose use.
//!
//! Backend implementations may define their own scheduling policies
//! (e.g., work-stealing, priority-based) as extensions.

pub(crate) struct _Phantom;
