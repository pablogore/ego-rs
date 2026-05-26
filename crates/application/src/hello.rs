use ego_domain::Query;

/// Error type for hello handler operations.
#[derive(Debug)]
pub struct HelloError(String);

impl std::fmt::Display for HelloError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for HelloError {}

/// Handler for the hello endpoint.
pub struct HelloHandler;

impl HelloHandler {
    pub fn handle(&self, _query: &impl Query) -> Result<String, HelloError> {
        Ok("Hello from ego-rs!".to_string())
    }
}
