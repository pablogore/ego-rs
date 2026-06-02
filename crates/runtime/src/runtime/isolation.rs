//! Isolation strategies for executions.
//!
//! This module is a documentation-only contract. The actual isolation
//! mechanism is left to backend implementations.
//!
//! # Contract
//!
//! - **Failure isolation**: A failure in one execution must not affect
//!   other executions.
//! - **Sequential per-unit**: Messages for a single execution are
//!   processed sequentially.
//!
//! Backend implementations may use threads, processes, or other
//! isolation mechanisms as appropriate.

pub(crate) struct _Phantom;
