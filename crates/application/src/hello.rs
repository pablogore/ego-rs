//! Hello handler — reference implementation of a QueryHandler.
//!
//! Processes `HelloQuery` and returns a `HelloResponse` with a static
//! greeting. Demonstrates the hexagonal flow: domain defines the query,
//! application implements the handler.

use ego_domain::hello::{HelloQuery, HelloResponse};

use crate::ports::QueryHandler;

/// Error returned by [`HelloHandler`].
#[derive(Debug)]
pub struct HelloError(String);

impl std::fmt::Display for HelloError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for HelloError {}

/// A handler that responds to [`HelloQuery`] with a greeting.
///
/// This is the reference implementation of a [`QueryHandler`].
/// It demonstrates the hexagonal flow: domain defines the query,
/// application implements the handler, transport exposes via HTTP/gRPC.
///
/// # Example
///
/// ```rust,ignore
/// use ego_domain::hello::HelloQuery;
/// use ego_application::ports::QueryHandler;
/// use ego_application::hello::HelloHandler;
///
/// let handler = HelloHandler;
/// let result = QueryHandler::handle(&handler, &HelloQuery);
/// assert_eq!(result.unwrap().message, "Hello from ego-rs!");
/// ```
pub struct HelloHandler;

impl QueryHandler<HelloQuery> for HelloHandler {
    type Error = HelloError;

    fn handle(&self, _query: &HelloQuery) -> Result<HelloResponse, Self::Error> {
        Ok(HelloResponse {
            message: "Hello from ego-rs!".to_string(),
        })
    }
}
