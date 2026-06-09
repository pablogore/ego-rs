use serde::{Deserialize, Serialize};

/// An error category.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
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

/// A domain error.
///
/// This trait should be implemented by all domain-specific errors.
pub trait DomainError: std::error::Error {
    /// Returns the error code for this error.
    fn code(&self) -> &str;

    /// Returns the error category for this error.
    fn category(&self) -> ErrorCategory;
}
