use serde::{Deserialize, Serialize};

/// A domain error trait.
///
/// This trait should be implemented by all domain-specific errors.
pub trait DomainError: std::error::Error {
    /// Returns the error code for this error.
    fn code(&self) -> &str;
    
    /// Returns the error category for this error.
    fn category(&self) -> ErrorCategory;
}

/// A trait for converting types into service errors.
pub trait IntoServiceError {
    /// Converts this type into a service error.
    fn into_service_error(self) -> ServiceError;
}