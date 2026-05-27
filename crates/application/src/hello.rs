use ego_domain::Query;

/// Error returned by [`HelloHandler`].
///
/// Wraps a human-readable error description. In production,
/// replace with a proper error enum.
#[derive(Debug)]
pub struct HelloError(String);

impl std::fmt::Display for HelloError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for HelloError {}

/// A handler that responds to [`HelloQuery`](ego_domain::hello::HelloQuery)
/// with a greeting.
///
/// This is the reference implementation of a [`QueryHandler`](crate::ports::QueryHandler).
/// It demonstrates the hexagonal flow: domain defines the query, application
/// implements the handler, transport exposes it via HTTP/gRPC.
///
/// # Example
///
/// ```rust,ignore
/// use ego_domain::hello::HelloQuery;
///
/// let handler = HelloHandler;
/// let result = handler.handle(&HelloQuery);
/// assert_eq!(result.unwrap(), "Hello from ego-rs!");
/// ```
pub struct HelloHandler;

impl HelloHandler {
    /// Process a hello query. Returns a static greeting.
    ///
    /// In a real handler, this would likely fetch data from a projection
    /// or read model through a port.
    pub fn handle(&self, _query: &impl Query) -> Result<String, HelloError> {
        Ok("Hello from ego-rs!".to_string())
    }
}