use std::fmt::Debug;

/// The scheduling policy for actor execution.
#[derive(Debug, Clone, Send, Sync)]
pub enum SchedulingPolicy {
    /// The actor is scheduled to run immediately.
    Immediate,
    /// The actor is scheduled to run after a delay.
    Delayed(u64),
    /// The actor is scheduled to run periodically.
    Periodic(u64),
}
