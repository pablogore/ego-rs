//! Marker traits for the CQRS command pattern.
//!
//! Commands represent intention to mutate state. They are processed by
//! `CommandHandler` implementations in the application layer. A command
//! is a plain data structure (no behavior), and the handler owns the
//! side effects.
//!
//! ```rust,ignore
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