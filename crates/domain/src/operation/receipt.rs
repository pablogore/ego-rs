//! The durable record that one operation already ran to completion.
//!
//! A receipt is written inside the same transaction as the events the operation
//! produced, which is what makes "did this already happen?" answerable without a
//! second source of truth that can disagree with the event stream.
//!
//! It is deliberately per-aggregate rather than global. The same operation key
//! addressed at two different aggregates is two operations, and collapsing them
//! would make one aggregate's completion suppress another's work.

use crate::context::TenantId;
use crate::operation::key::{OperationFingerprint, OperationKey};
use crate::operation::reservation::StoredResponse;

/// The record confirming that one operation completed against one aggregate.
///
/// # Why the fingerprint travels with the receipt
///
/// Replaying a stored response is only safe when the retry is the *same*
/// request. The fingerprint is what distinguishes a genuine retry — which must
/// replay — from a different command reusing an operation key, which must be
/// refused rather than answered with someone else's result.
///
/// # Why zero events still produces one
///
/// A command that succeeds without emitting anything has no event to carry its
/// completion, so without a receipt it is indistinguishable from a command that
/// never ran. That case is normative, not an edge: it is the reason the receipt
/// lives in its own table rather than as columns on `events`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OperationReceipt {
    aggregate_type: String,
    aggregate_id: String,
    tenant: Option<TenantId>,
    operation_key: OperationKey,
    fingerprint: OperationFingerprint,
    response: StoredResponse,
}

impl OperationReceipt {
    /// Builds a receipt for one completed operation against one aggregate.
    ///
    /// `tenant` is `None` for the systemwide scope, matching the same
    /// three-valued partitioning the event store uses.
    pub fn new(
        aggregate_type: impl Into<String>,
        aggregate_id: impl Into<String>,
        tenant: Option<TenantId>,
        operation_key: OperationKey,
        fingerprint: OperationFingerprint,
        response: StoredResponse,
    ) -> Self {
        Self {
            aggregate_type: aggregate_type.into(),
            aggregate_id: aggregate_id.into(),
            tenant,
            operation_key,
            fingerprint,
            response,
        }
    }

    /// The aggregate type this operation ran against.
    pub fn aggregate_type(&self) -> &str {
        &self.aggregate_type
    }

    /// The aggregate instance this operation ran against.
    pub fn aggregate_id(&self) -> &str {
        &self.aggregate_id
    }

    /// The resolved tenant scope, or `None` for the systemwide scope.
    pub fn tenant(&self) -> Option<&TenantId> {
        self.tenant.as_ref()
    }

    /// The operation key this receipt answers for.
    pub fn operation_key(&self) -> &OperationKey {
        &self.operation_key
    }

    /// The fingerprint of the request that produced this receipt.
    pub fn fingerprint(&self) -> &OperationFingerprint {
        &self.fingerprint
    }

    /// The stored response to replay for a matching retry.
    pub fn response(&self) -> &StoredResponse {
        &self.response
    }
}
