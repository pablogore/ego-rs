//! Hexagonal port traits for the application layer.
//!
//! Defines `CommandHandler` and `QueryHandler` — the ports that transport
//! adapters (HTTP, gRPC) call and that application handlers implement.
//! Both depend only on domain traits.

use ego_domain::{Command, Query};

/// Trait for command handlers in the application layer.
///
/// A command handler receives a [`Command`] and produces either `Ok(())`
/// or an application-specific error. Handlers own validation, business
/// rules, and side-effect orchestration.
///
/// # Example
///
/// ```rust
/// use ego_application::ports::CommandHandler;
///
/// // This shows the pattern - actual implementation would depend on specific command type
/// struct MyCommandHandler;
///
/// // impl CommandHandler<SomeCommandType> for MyCommandHandler { ... }
/// ```
pub trait CommandHandler<C: Command> {
    /// The error type returned on handler failure.
    type Error;

    /// Process the command. Returns `Ok(())` on success or an error.
    fn handle(&self, command: &C) -> Result<(), Self::Error>;
}

/// Trait for query handlers in the application layer.
///
/// A query handler receives a [`Query`] and returns its typed [`Query::Output`].
/// Queries are read-only — they never mutate state.
///
/// # Example
///
/// ```rust
/// use ego_application::ports::QueryHandler;
///
/// // This shows the pattern - actual implementation would depend on specific query type
/// struct MyQueryHandler;
///
/// // impl QueryHandler<SomeQueryType> for MyQueryHandler { ... }
/// ```
pub trait QueryHandler<Q: Query> {
    /// The error type returned on handler failure.
    type Error;

    /// Process the query. Returns the typed output or an error.
    fn handle(&self, query: &Q) -> Result<Q::Output, Self::Error>;
}
