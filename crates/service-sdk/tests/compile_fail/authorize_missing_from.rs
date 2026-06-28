// Fixture: E_from — error type does not implement From<SecurityError>.
// Linked requirement: FR-6, AC-6.1.
use ego_service_sdk::context::ServiceContext;
use ego_service_sdk::error::category::ErrorCategory;
use ego_service_sdk::error::ServiceErrorTrait;
use ego_service_sdk_macros::{authorize, operation, service};

// Deliberately does NOT implement From<SecurityError>.
#[derive(Debug)]
pub struct NoFromError(String);

impl ServiceErrorTrait for NoFromError {
    fn code(&self) -> &str {
        "NO_FROM_ERROR"
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
    async fn get_order(&self, ctx: ServiceContext, id: String) -> Result<String, NoFromError>;
}

fn main() {}
