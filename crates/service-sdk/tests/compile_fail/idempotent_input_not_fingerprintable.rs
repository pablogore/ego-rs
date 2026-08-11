// Fixture: an `#[idempotent]` operation whose argument has no canonical form.
//
// Both other obligations are satisfied deliberately — the output round-trips
// and the error absorbs a refused reservation — so this can only fail for the
// input bound. Without that isolation the fixture would stay green if the input
// bound were silently dropped, which is the failure mode these three fixtures
// exist to make impossible.
//
// Why the obligation exists: the reservation is keyed by a fingerprint computed
// over the operation's typed arguments (AD-3f). An argument with no canonical
// form cannot be fingerprinted, so the operation could only be reserved under
// something that ignores it — and two genuinely different requests would then
// deduplicate against each other, one silently replaying the other's answer.
use ego_service_sdk::context::ServiceContext;
use ego_service_sdk::error::category::ErrorCategory;
use ego_service_sdk::error::ServiceErrorTrait;
use ego_service_sdk::runtime::ReservationRejection;
use ego_service_sdk_macros::service;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
pub struct PlainReceipt {
    pub id: String,
}

/// No `Serialize`.
pub struct OpaqueRequest {
    pub id: String,
}

#[derive(Debug)]
pub struct PlainError(String);

impl From<ReservationRejection> for PlainError {
    fn from(r: ReservationRejection) -> Self {
        PlainError(r.to_string())
    }
}

impl ServiceErrorTrait for PlainError {
    fn code(&self) -> &str {
        "PLAIN_ERROR"
    }
    fn category(&self) -> ErrorCategory {
        ErrorCategory::Business
    }
    fn message(&self) -> String {
        self.0.clone()
    }
}

#[service(version = "1.0.0")]
pub trait OpaqueInputService {
    #[operation]
    #[idempotent]
    async fn charge(
        &self,
        ctx: ServiceContext,
        request: OpaqueRequest,
    ) -> Result<PlainReceipt, PlainError>;
}

fn main() {}
