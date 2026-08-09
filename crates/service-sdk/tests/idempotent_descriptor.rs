//! PROD-012 B6.2 — `#[idempotent]` must reach the generated contract.
//!
//! `OperationDescriptor::idempotent` already existed before this change and was
//! written as a literal `false` for every operation the `#[service]` generator
//! emitted. The field was serialised, exposed through `ServiceContract`, and
//! read by anyone introspecting a service — while describing nothing that the
//! code actually did.
//!
//! That is the failure this test exists to prevent: a declaration that looks
//! authoritative and is never populated. Marking an operation `#[idempotent]`
//! must be visible in its descriptor, and leaving it unmarked must remain
//! visible as `false` — otherwise the flag is decoration and a consumer acting
//! on it is acting on a guess.

use ego_service_sdk::context::ServiceContext;
use ego_service_sdk::contract::ServiceContract;
use ego_service_sdk::error::category::ErrorCategory;
use ego_service_sdk::error::ServiceErrorTrait;
use ego_service_sdk_macros::service;

#[derive(Debug)]
pub struct DescriptorError(String);

impl ServiceErrorTrait for DescriptorError {
    fn code(&self) -> &str {
        "DESCRIPTOR_ERROR"
    }
    fn category(&self) -> ErrorCategory {
        ErrorCategory::Business
    }
    fn message(&self) -> String {
        self.0.clone()
    }
}

#[service(version = "1.0.0")]
pub trait BillingService {
    /// Marked: retrying this must be safe, and the contract must say so.
    #[operation]
    #[idempotent]
    async fn charge(&self, ctx: ServiceContext, id: String) -> Result<String, DescriptorError>;

    /// Unmarked: the contract must keep reporting `false` rather than
    /// defaulting everything to idempotent once the field became writable.
    #[operation]
    async fn audit(&self, ctx: ServiceContext, id: String) -> Result<String, DescriptorError>;
}

fn descriptor_for(name: &str) -> ego_service_sdk::contract::OperationDescriptor {
    <BillingServiceTag as ServiceContract>::operations()
        .into_iter()
        .find(|op| op.name == name)
        .unwrap_or_else(|| panic!("the generated contract must describe `{name}`"))
}

#[test]
fn a_marked_operation_is_reported_as_idempotent() {
    assert!(
        descriptor_for("charge").idempotent,
        "#[idempotent] must reach the generated contract: the field was a hardcoded \
         `false` before this change, so a consumer reading it learned nothing about \
         the operation"
    );
}

#[test]
fn an_unmarked_operation_is_still_reported_as_not_idempotent() {
    assert!(
        !descriptor_for("audit").idempotent,
        "an operation nobody marked must not become idempotent by default — the flag \
         has to distinguish the two, not merely be writable"
    );
}
