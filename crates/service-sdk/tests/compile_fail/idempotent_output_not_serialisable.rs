// Fixture: an `#[idempotent]` operation whose output cannot round-trip.
//
// The error type DOES implement `From<ReservationRejection>`, deliberately. If
// it did not, this fixture would fail for that reason instead and would prove
// nothing about the serde obligation — a fixture must satisfy every requirement
// it is not testing.
//
// Why the obligation exists: a replayed reservation answers with bytes the
// runtime stored, so an output the runtime can write but not read back would
// leave a completed operation unanswerable.
use ego_service_sdk::context::ServiceContext;
use ego_service_sdk::error::category::ErrorCategory;
use ego_service_sdk::error::ServiceErrorTrait;
use ego_service_sdk::runtime::ReservationRejection;
use ego_service_sdk_macros::service;

/// No `Serialize`, no `Deserialize`.
pub struct OpaqueReceipt {
    pub id: String,
}

#[derive(Debug)]
pub struct OpaqueError(String);

impl From<ReservationRejection> for OpaqueError {
    fn from(r: ReservationRejection) -> Self {
        OpaqueError(r.to_string())
    }
}

impl ServiceErrorTrait for OpaqueError {
    fn code(&self) -> &str {
        "OPAQUE_ERROR"
    }
    fn category(&self) -> ErrorCategory {
        ErrorCategory::Business
    }
    fn message(&self) -> String {
        self.0.clone()
    }
}

#[service(version = "1.0.0")]
pub trait OpaqueService {
    #[operation]
    #[idempotent]
    async fn charge(
        &self,
        ctx: ServiceContext,
        id: String,
    ) -> Result<OpaqueReceipt, OpaqueError>;
}

fn main() {}
