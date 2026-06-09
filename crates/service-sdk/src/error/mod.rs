pub mod category;

use serde::{Deserialize, Serialize};

/// A service error that represents various failure conditions in service operations.
///
/// Service errors are categorized to provide structured error handling and
/// appropriate responses to service callers. Each error variant represents a
/// specific type of failure that can occur during service execution.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ServiceError {
    /// A validation error indicating that input data failed validation checks.
    Validation {
        /// The error message describing the validation failure.
        message: String,
    },
    /// An authorization error indicating that the caller lacks necessary permissions.
    Authorization {
        /// The error message describing the authorization failure.
        message: String,
    },
    /// An internal error indicating an unexpected system failure.
    Internal {
        /// The error message describing the internal error.
        message: String,
    },
    /// A not found error indicating that a requested resource was not found.
    NotFound {
        /// The error message describing the resource not found error.
        message: String,
    },
    /// A conflict error indicating that the operation conflicts with the current state.
    Conflict {
        /// The error message describing the conflict.
        message: String,
    },
    /// A timeout error indicating that the operation exceeded its time limit.
    Timeout {
        /// The error message describing the timeout.
        message: String,
    },
    /// A rate limit error indicating that the caller has exceeded rate limits.
    RateLimit {
        /// The error message describing the rate limit violation.
        message: String,
    },
    /// A service unavailable error indicating that the service is temporarily unavailable.
    ServiceUnavailable {
        /// The error message describing the service unavailability.
        message: String,
    },
    /// A business logic error indicating that the operation failed due to business rules.
    BusinessLogic {
        /// The error message describing the business logic failure.
        message: String,
    },
    /// A custom error for application-specific error conditions.
    Custom {
        /// The error message describing the custom error.
        message: String,
    },
}

impl ServiceError {
    /// Creates a new validation error.
    ///
    /// # Arguments
    /// * `message` - A description of the validation failure
    ///
    /// # Returns
    /// A `ServiceError::Validation` variant
    pub fn validation(message: impl Into<String>) -> Self {
        ServiceError::Validation {
            message: message.into(),
        }
    }

    /// Creates a new authorization error.
    ///
    /// # Arguments
    /// * `message` - A description of the authorization failure
    ///
    /// # Returns
    /// A `ServiceError::Authorization` variant
    pub fn authorization(message: impl Into<String>) -> Self {
        ServiceError::Authorization {
            message: message.into(),
        }
    }

    /// Creates a new internal error.
    ///
    /// # Arguments
    /// * `message` - A description of the internal error
    ///
    /// # Returns
    /// A `ServiceError::Internal` variant
    pub fn internal(message: impl Into<String>) -> Self {
        ServiceError::Internal {
            message: message.into(),
        }
    }

    /// Creates a new not found error.
    ///
    /// # Arguments
    /// * `message` - A description of the resource not found error
    ///
    /// # Returns
    /// A `ServiceError::NotFound` variant
    pub fn not_found(message: impl Into<String>) -> Self {
        ServiceError::NotFound {
            message: message.into(),
        }
    }

    /// Creates a new conflict error.
    ///
    /// # Arguments
    /// * `message` - A description of the conflict
    ///
    /// # Returns
    /// A `ServiceError::Conflict` variant
    pub fn conflict(message: impl Into<String>) -> Self {
        ServiceError::Conflict {
            message: message.into(),
        }
    }

    /// Creates a new timeout error.
    ///
    /// # Arguments
    /// * `message` - A description of the timeout
    ///
    /// # Returns
    /// A `ServiceError::Timeout` variant
    pub fn timeout(message: impl Into<String>) -> Self {
        ServiceError::Timeout {
            message: message.into(),
        }
    }

    /// Creates a new rate limit error.
    ///
    /// # Arguments
    /// * `message` - A description of the rate limit violation
    ///
    /// # Returns
    /// A `ServiceError::RateLimit` variant
    pub fn rate_limit(message: impl Into<String>) -> Self {
        ServiceError::RateLimit {
            message: message.into(),
        }
    }

    /// Creates a new service unavailable error.
    ///
    /// # Arguments
    /// * `message` - A description of the service unavailability
    ///
    /// # Returns
    /// A `ServiceError::ServiceUnavailable` variant
    pub fn service_unavailable(message: impl Into<String>) -> Self {
        ServiceError::ServiceUnavailable {
            message: message.into(),
        }
    }

    /// Creates a new business logic error.
    ///
    /// # Arguments
    /// * `message` - A description of the business logic failure
    ///
    /// # Returns
    /// A `ServiceError::BusinessLogic` variant
    pub fn business_logic(message: impl Into<String>) -> Self {
        ServiceError::BusinessLogic {
            message: message.into(),
        }
    }

    /// Creates a new custom error.
    ///
    /// # Arguments
    /// * `message` - A description of the custom error
    ///
    /// # Returns
    /// A `ServiceError::Custom` variant
    pub fn custom(message: impl Into<String>) -> Self {
        ServiceError::Custom {
            message: message.into(),
        }
    }
}

/// A service result type alias for consistent error handling.
///
/// This type alias provides a convenient way to work with service operations
/// that may return either a successful result or a service error.
pub type Result<T> = std::result::Result<T, ServiceError>;

impl From<crate::error::category::ErrorCategory> for ServiceError {
    fn from(category: crate::error::category::ErrorCategory) -> Self {
        match category {
            crate::error::category::ErrorCategory::Validation => ServiceError::Validation {
                message: "validation error".to_string(),
            },
            crate::error::category::ErrorCategory::Authorization => ServiceError::Authorization {
                message: "authorization error".to_string(),
            },
            crate::error::category::ErrorCategory::System => ServiceError::Internal {
                message: "system error".to_string(),
            },
            crate::error::category::ErrorCategory::Network => ServiceError::Internal {
                message: "network error".to_string(),
            },
            crate::error::category::ErrorCategory::Authentication => ServiceError::Authorization {
                message: "authentication error".to_string(),
            },
            crate::error::category::ErrorCategory::Resource => ServiceError::NotFound {
                message: "resource error".to_string(),
            },
            crate::error::category::ErrorCategory::Timeout => ServiceError::Timeout {
                message: "timeout error".to_string(),
            },
            crate::error::category::ErrorCategory::Unknown => ServiceError::Internal {
                message: "unknown error".to_string(),
            },
            crate::error::category::ErrorCategory::Business => ServiceError::BusinessLogic {
                message: "business logic error".to_string(),
            },
        }
    }
}
