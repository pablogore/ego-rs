// Fixture: `#[idempotent]` on a `#[service]` method that is not an
// `#[operation]`.
//
// The marker only means something on a dispatched operation: the reservation
// slot it enables runs in the generated operation path, and a method outside
// that path never reaches it. Accepting the annotation there would record an
// idempotency promise on a method whose calls are never reserved, replayed, or
// refused — the promise would be real to every reader and enforced by nothing.
use ego_service_sdk::context::ServiceContext;
use ego_service_sdk::error::category::ErrorCategory;
use ego_service_sdk::error::ServiceErrorTrait;
use ego_service_sdk_macros::{idempotent, operation, service};

#[derive(Debug)]
pub struct MarkerError(String);

impl ServiceErrorTrait for MarkerError {
    fn code(&self) -> &str {
        "MARKER_ERROR"
    }
    fn category(&self) -> ErrorCategory {
        ErrorCategory::Business
    }
    fn message(&self) -> String {
        self.0.clone()
    }
}

#[service(version = "1.0.0")]
pub trait MarkerService {
    #[operation]
    async fn dispatched(&self, ctx: ServiceContext, id: String) -> Result<String, MarkerError>;

    #[idempotent]
    async fn not_an_operation(&self, ctx: ServiceContext, id: String)
        -> Result<String, MarkerError>;
}

fn main() {}
