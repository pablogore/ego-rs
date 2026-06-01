use std::fmt::Debug;

/// The isolation strategy for an actor.
#[derive(Debug, Clone)]
pub enum Isolation {
    /// The actor runs in a separate thread.
    Thread,
    /// The actor runs in a separate process.
    Process,
    /// The actor runs in a separate container.
    Container,
}
