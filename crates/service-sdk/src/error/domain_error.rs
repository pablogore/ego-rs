//! Canonical definition of the `DomainError` trait.

use crate::error::category::ErrorCategory;

/// A domain error trait.
///
/// This is the single canonical definition of DomainError.
/// All domain-specific errors should implement this trait.
pub trait DomainError: std::error::Error + Send + Sync {
    /// Returns the error code for this error.
    fn code(&self) -> &str;

    /// Returns the error category for this error.
    fn category(&self) -> ErrorCategory;
}

/// A trait for converting types into service errors.
pub trait IntoServiceError {
    /// Converts this type into a service error.
    fn into_service_error(self) -> crate::error::ServiceError;
}
