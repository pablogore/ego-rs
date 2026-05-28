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
}
