// Compile-pass smoke test: a valid `#[authorize]` annotation on a `#[service]` method.
//
// AC-1.1: `#[authorize(context = ctx, permission = "orders:read")]` on a service method
// inside `#[service]` compiles and generates an authorization guard.
// FR-6: The error type `AuthOrderError` implements `From<SecurityError>`, satisfying the bound.

use ego_security_sdk::SecurityError;
use ego_service_sdk::context::ServiceContext;
use ego_service_sdk::error::category::ErrorCategory;
use ego_service_sdk::error::ServiceErrorTrait;
use ego_service_sdk_macros::{authorize, operation, service};

#[derive(Debug)]
pub struct AuthOrderError(String);

impl From<SecurityError> for AuthOrderError {
    fn from(e: SecurityError) -> Self {
        AuthOrderError(e.to_string())
    }
}

impl ServiceErrorTrait for AuthOrderError {
    fn code(&self) -> &str {
        "AUTH_ORDER_ERROR"
    }
    fn category(&self) -> ErrorCategory {
        ErrorCategory::Business
    }
    fn message(&self) -> String {
        self.0.clone()
    }
}

#[service(version = "1.0.0")]
pub trait OrderService {
    #[operation]
    #[authorize(context = ctx, permission = "orders:read")]
    async fn get_order(&self, ctx: ServiceContext, id: String) -> Result<String, AuthOrderError>;
}

fn main() {}
