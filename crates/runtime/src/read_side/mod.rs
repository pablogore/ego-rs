//! Read side projection runtime.
//!
//! Provides async orchestration for tag-based projections:
//! scheduling, backpressure, and batch execution.

pub mod backpressure;
pub mod batch_executor;
pub mod scheduler;

pub use scheduler::{ReadSideProjectionHandle, ReadSideStopOutcome};
