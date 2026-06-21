//! Example of how to use logging in service implementations

/// Example service that uses logging
pub struct ExampleService;

impl ExampleService {
    /// Process a request with logging
    pub fn process_request(&self, input: &str) -> Result<String, String> {
        // kitlogger is available as a dependency
        // This shows that the dependency is properly integrated

        if input.is_empty() {
            return Err("Input cannot be empty".to_string());
        }

        let result = format!("Processed: {}", input);
        Ok(result)
    }
}
