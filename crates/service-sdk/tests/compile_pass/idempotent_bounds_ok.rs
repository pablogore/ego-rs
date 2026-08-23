// Compile-pass: an `#[idempotent]` operation whose output round-trips and whose
// error absorbs a refused reservation.
//
// Both obligations are satisfied here, so the pair is the control for the two
// negative fixtures: each of those drops exactly one of them.
use ego_service_sdk::context::ServiceContext;
use ego_service_sdk::error::category::ErrorCategory;
use ego_service_sdk::error::ServiceErrorTrait;
use ego_service_sdk::runtime::ReservationRejection;
use ego_service_sdk_macros::service;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
pub struct ChargeReceipt {
    pub id: String,
}

#[derive(Debug)]
pub struct ChargeError(String);

impl From<ReservationRejection> for ChargeError {
    fn from(r: ReservationRejection) -> Self {
        ChargeError(r.to_string())
    }
}

impl ServiceErrorTrait for ChargeError {
    fn code(&self) -> &str {
        "CHARGE_ERROR"
    }
    fn category(&self) -> ErrorCategory {
        ErrorCategory::Business
    }
    fn message(&self) -> String {
        self.0.clone()
    }
}

#[service(version = "1.0.0")]
pub trait ChargeService {
    #[operation]
    #[idempotent]
    async fn charge(
        &self,
        ctx: ServiceContext,
        id: String,
    ) -> Result<ChargeReceipt, ChargeError>;
}

fn main() {}
