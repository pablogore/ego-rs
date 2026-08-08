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

/// The durable result of one command's transition against one aggregate.
///
/// This is **not** the service operation's response. A service operation may
/// command several aggregates and compose its answer from all of them; that
/// composed answer is a [`StoredServiceResponse`] and belongs to the
/// reservation, written by the idempotency slot after the handler returns. The
/// two may share a byte representation downstream; they share neither scope,
/// semantics, nor owner, and one name for both is how they were conflated once
/// already.
///
/// [`StoredServiceResponse`]: crate::operation::StoredServiceResponse
///
/// # Why a range and not a copy
///
/// The events are already durable, committed in the same unit of work as this
/// record. Storing them again would duplicate the stream and force a
/// `Serialize` bound onto every domain event in the workspace to record what
/// the stream already holds. The resulting state is redundant for the same
/// reason: a replay rebuilds it from those very events, and a stored copy is a
/// second answer that can fall out of step with the first.
///
/// What cannot be recovered from the stream alone is *which* slice of it this
/// command produced. That is all this type carries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AggregateOutcome {
    /// The command appended this inclusive version range.
    Events {
        /// First version this command wrote. Inclusive.
        version_from: i64,
        /// Last version this command wrote. Inclusive, and never below
        /// `version_from`.
        version_to: i64,
    },
    /// The command succeeded and appended nothing.
    ///
    /// This is the case the receipt exists for: a success with no event has
    /// nothing in the stream to carry its completion, so without this record it
    /// is indistinguishable from a command that never ran.
    ///
    /// It is also the **only** encoding of an empty range. An `Events` range
    /// that describes nothing is not constructible, so the two can never both
    /// mean "nothing happened".
    NoEvents,
}

/// Rejected constructions of an [`AggregateOutcome::Events`] range.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AggregateOutcomeError {
    /// `version_to` is below `version_from`.
    Inverted,
    /// The range would describe no events, which only [`AggregateOutcome::NoEvents`]
    /// may express.
    Empty,
}

impl AggregateOutcome {
    /// Builds an inclusive event range, refusing the two shapes that would make
    /// a receipt ambiguous.
    ///
    /// The check lives in the constructor rather than at the read site because a
    /// range is validated once when written and trusted on every replay
    /// afterwards; a reader that has to re-validate is a reader that can forget.
    pub fn events(version_from: i64, version_to: i64) -> Result<Self, AggregateOutcomeError> {
        // `Empty` is checked first, and the order is load-bearing rather than
        // stylistic: `to == from - 1` is itself a case of `to < from`, so
        // testing `Inverted` first would make `Empty` unreachable and report an
        // off-by-one at the call site as swapped bounds. The two call for
        // different fixes, which is the only reason they are separate variants.
        if version_to == version_from - 1 {
            return Err(AggregateOutcomeError::Empty);
        }
        if version_to < version_from {
            return Err(AggregateOutcomeError::Inverted);
        }
        Ok(Self::Events {
            version_from,
            version_to,
        })
    }
}

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
    outcome: AggregateOutcome,
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
        outcome: AggregateOutcome,
    ) -> Self {
        Self {
            aggregate_type: aggregate_type.into(),
            aggregate_id: aggregate_id.into(),
            tenant,
            operation_key,
            fingerprint,
            outcome,
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

    /// The durable transition a matching retry replays instead of re-running.
    pub fn outcome(&self) -> &AggregateOutcome {
        &self.outcome
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The ordinary shape: a command that appended versions 4 through 6.
    #[test]
    fn an_inclusive_ascending_range_is_accepted() {
        let outcome = AggregateOutcome::events(4, 6).expect("4..=6 is a valid range");
        assert_eq!(
            outcome,
            AggregateOutcome::Events {
                version_from: 4,
                version_to: 6
            }
        );
    }

    /// One event is a range of one, not a degenerate case.
    #[test]
    fn a_single_event_range_is_accepted() {
        assert!(AggregateOutcome::events(7, 7).is_ok());
    }

    /// `NoEvents` is the only encoding of an empty range.
    ///
    /// Without this, a receipt could say "nothing happened" in two different
    /// ways, and a replay would have to decide which one it was looking at.
    #[test]
    fn an_empty_range_is_refused_because_no_events_already_means_that() {
        assert_eq!(
            AggregateOutcome::events(5, 4),
            Err(AggregateOutcomeError::Empty)
        );
    }

    /// An inverted range describes no slice of any stream, so it can only be a
    /// writer that computed its bounds the wrong way round. Accepting it would
    /// store a receipt no replay can ever satisfy.
    #[test]
    fn an_inverted_range_is_refused() {
        assert_eq!(
            AggregateOutcome::events(9, 3),
            Err(AggregateOutcomeError::Inverted)
        );
    }

    /// The two refusals are distinguishable, because they call for different
    /// fixes: an off-by-one at the call site versus swapped bounds.
    #[test]
    fn the_two_refusals_are_distinct() {
        assert_ne!(
            AggregateOutcomeError::Empty,
            AggregateOutcomeError::Inverted
        );
    }
}
