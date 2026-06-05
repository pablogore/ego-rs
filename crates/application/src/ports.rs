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
/// ```rust,ignore
/// use ego_domain::Command;
/// use ego_application::ports::CommandHandler;
///
/// struct CreateUserHandler;
///
/// impl CommandHandler<CreateUser> for CreateUserHandler {
///     type Error = String;
///     fn handle(&self, cmd: &CreateUser) -> Result<(), Self::Error> {
///         // validate, save, publish events...
///         Ok(())
///     }
/// }
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
/// ```rust,ignore
/// use ego_domain::Query;
/// use ego_application::ports::QueryHandler;
///
/// struct GetUserHandler;
///
/// impl QueryHandler<GetUser> for GetUserHandler {
///     type Error = String;
///     fn handle(&self, query: &GetUser) -> Result<UserProfile, Self::Error> {
///         // fetch from read model...
///         Ok(UserProfile { name: "Alice".into() })
///     }
/// }
/// ```
pub trait QueryHandler<Q: Query> {
    /// The error type returned on handler failure.
    type Error;

    /// Process the query. Returns the typed output or an error.
    fn handle(&self, query: &Q) -> Result<Q::Output, Self::Error>;
}
