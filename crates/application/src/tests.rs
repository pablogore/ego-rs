#[cfg(test)]
mod tests {
    use ego_application::hello::HelloHandler;
    use ego_application::ports::QueryHandler;
    use ego_domain::hello::{HelloQuery, HelloResponse};

    #[test]
    fn test_deterministic_execution() {
        let handler = HelloHandler;
        let query = HelloQuery;

        // First execution
        let result1 = QueryHandler::handle(&handler, &query).unwrap();
        assert_eq!(result1.message, "Hello from ego-rs!");

        // Second execution with identical input
        let result2 = QueryHandler::handle(&handler, &query).unwrap();
        assert_eq!(result2.message, "Hello from ego-rs!");

        // Verify identical observable semantics
        assert_eq!(result1, result2);
    }

    #[test]
    fn test_executor_deterministic() {
        // Define a simple command and query for testing
        struct TestCommand;
        impl Command for TestCommand {}

        struct TestQuery;
        impl Query for TestQuery {
            type Output = String;
        }

        // Define a simple handler for the test command and query
        struct TestHandler;
        impl CommandHandler<TestCommand> for TestHandler {
            type Error = String;

            fn handle(&self, _command: &TestCommand) -> Result<(), Self::Error> {
                Ok(())
            }
        }

        impl QueryHandler<TestQuery> for TestHandler {
            type Error = String;

            fn handle(&self, _query: &TestQuery) -> Result<Self::Output, Self::Error> {
                Ok("Test response".to_string())
            }
        }

        // Create an instance of the handler
        let handler = TestHandler;

        // First execution
        let result1 = QueryHandler::handle(&handler, &TestQuery).unwrap();
        assert_eq!(result1, "Test response");

        // Second execution with identical input
        let result2 = QueryHandler::handle(&handler, &TestQuery).unwrap();
        assert_eq!(result2, "Test response");

        // Verify identical observable semantics
        assert_eq!(result1, result2);
    }
}
