use ego_domain::{Command, Query};

/// Trait for command handlers in the application layer.
pub trait CommandHandler<C: Command> {
    type Error;
    fn handle(&self, command: &C) -> Result<(), Self::Error>;
}

/// Trait for query handlers in the application layer.
pub trait QueryHandler<Q: Query> {
    type Error;
    fn handle(&self, query: &Q) -> Result<Q::Output, Self::Error>;
}
