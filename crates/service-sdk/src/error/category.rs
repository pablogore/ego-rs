//! Error category enumeration — single canonical definition.

/// An error category for structured error handling and routing.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ErrorCategory {
    /// A validation error.
    Validation,
    /// A business logic error.
    Business,
    /// A system error.
    System,
    /// A network error.
    Network,
    /// An authentication error.
    Authentication,
    /// An authorization error.
    Authorization,
    /// A resource error.
    Resource,
    /// A timeout error.
    Timeout,
    /// An unknown error.
    Unknown,
}
