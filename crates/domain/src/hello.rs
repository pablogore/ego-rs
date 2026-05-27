use serde::{Deserialize, Serialize};

use crate::query::Query;

/// A query requesting the hello greeting.
///
/// Uses the CQRS query pattern: `HelloQuery` implements `Query` with
/// `Output = HelloResponse`. Handlers in the application layer process
/// this query without mutating state.
///
/// # Example
///
/// ```rust,ignore
/// use ego_domain::hello::{HelloQuery, HelloResponse};
/// use ego_domain::Query;
///
/// let query = HelloQuery;
/// // The application layer resolves this via a QueryHandler<HelloQuery>
/// ```
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct HelloQuery;

/// The response to a [`HelloQuery`].
///
/// Contains the greeting message produced by the hello handler.
///
/// # Example
///
/// ```rust,ignore
/// let response = HelloResponse {
///     message: "Hello from ego-rs!".into(),
/// };
/// assert_eq!(response.message, "Hello from ego-rs!");
/// ```
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct HelloResponse {
    /// The greeting message returned by the handler.
    pub message: String,
}

impl Query for HelloQuery {
    type Output = HelloResponse;
}