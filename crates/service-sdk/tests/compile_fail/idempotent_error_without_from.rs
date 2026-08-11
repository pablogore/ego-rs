// Fixture: an `#[idempotent]` operation whose error cannot absorb a refused
// reservation.
//
// The output DOES satisfy both serde bounds, deliberately — otherwise this
// would fail for the other obligation and prove nothing about `From`.
//
// Why the obligation exists: a reservation that refuses is returned as the
// operation's own error, converted by `From`. Without it the macro would have to
// interpret the outcome itself, which AD-3g rejected.
use ego_service_sdk::context::ServiceContext;
use ego_service_sdk::error::category::ErrorCategory;
use ego_service_sdk::error::ServiceErrorTrait;
use ego_service_sdk_macros::service;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
pub struct PlainReceipt {
    pub id: String,
}

/// No `From<ReservationRejection>`.
#[derive(Debug)]
pub struct UnconvertibleError(String);

impl ServiceErrorTrait for UnconvertibleError {
    fn code(&self) -> &str {
        "UNCONVERTIBLE_ERROR"
    }
    fn category(&self) -> ErrorCategory {
        ErrorCategory::Business
    }
    fn message(&self) -> String {
        self.0.clone()
    }
}

#[service(version = "1.0.0")]
pub trait UnconvertibleService {
    #[operation]
    #[idempotent]
    async fn charge(
        &self,
        ctx: ServiceContext,
        id: String,
    ) -> Result<PlainReceipt, UnconvertibleError>;
}

fn main() {}
