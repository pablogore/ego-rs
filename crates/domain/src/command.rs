//! Marker traits for the CQRS command pattern.
//!
//! Commands represent intention to mutate state. They are processed by
//! `CommandHandler` implementations in the application layer. A command
//! is a plain data structure (no behavior), and the handler owns the
//! side effects.
//!
//! ```rust
//! use ego_domain::Command;
//!
//! #[derive(Debug)]
//! struct CreateUser { name: String }
//! impl Command for CreateUser {}
//! ```

/// Marker trait for command types.
///
/// Commands carry the data needed to perform a mutation. The handler
/// (in the application layer) owns: validation, business rules, and
/// side effects (events, persistence, etc.).
///
/// # Example
///
/// ```rust
/// use ego_domain::Command;
///
/// #[derive(Debug)]
/// struct PlaceOrder { item: String, quantity: u32 }
///
/// impl Command for PlaceOrder {}
/// ```
pub trait Command: Send + Sync {}

#[cfg(test)]
mod tests {
    use super::Command;

    /// A test command struct that implements the Command trait
    #[derive(Debug)]
    struct TestCommand {
        name: String,
    }

    impl Command for TestCommand {}

    /// A test command that should be valid
    #[test]
    fn test_command_trait_is_implemented() {
        let command = TestCommand {
            name: "test".to_string(),
        };

        // The fact that this compiles means the trait is properly implemented
        // We can't really test the trait itself since it's just a marker trait
        assert_eq!(command.name, "test");
    }

    /// Test that commands can be used in contexts that require the Command trait
    #[test]
    fn test_command_can_be_used_in_trait_context() {
        fn process_command<C: Command>(_command: &C) {
            // This function accepts any type that implements Command
            // The fact that we can pass a TestCommand means the trait is properly implemented
        }

        let command = TestCommand {
            name: "test".to_string(),
        };

        process_command(&command);
    }
}
