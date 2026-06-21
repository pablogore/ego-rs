//! Object-safe error trait that interceptors program against.

use crate::error::category::ErrorCategory;

/// An object-safe trait for service errors.
///
/// Interceptors receive `&dyn ServiceErrorTrait` instead of concrete error types,
/// allowing domain errors to flow through the interceptor chain unchanged while
/// still providing structured error information.
pub trait ServiceErrorTrait: Send + Sync {
    /// Returns a short machine-readable code identifying this error.
    fn code(&self) -> &str;

    /// Returns the error category for structured routing.
    fn category(&self) -> ErrorCategory;

    /// Returns a human-readable description of the error.
    fn message(&self) -> String;
}
