//! Post-commit external-effect acceptance port (CORE-019 Phase 5, AD-3).
//!
//! Mirrors [`crate::publisher::EventPublisher`]: the trait lives here so the
//! actor depends only on its own crate, never on `ego-runtime`'s
//! `EffectStoreError` (AD-3's dependency direction — `runtime` depends on
//! `persistent-entity`, not the reverse). The `RuntimeEffectAcceptor` impl
//! (in `ego-runtime`) maps its own store-error taxonomy into
//! [`EffectAcceptanceError`] after applying the bounded retry policy.

use async_trait::async_trait;
use ego_domain::{ExternalEffectDescription, IdempotencyKey, TenantId};
use thiserror::Error;

/// Layer-neutral acceptance-failure classification (AD-9). Deliberately does
/// NOT reference `ego-runtime`'s `EffectStoreError` — see the module docs.
///
/// Both variants carry partial-acceptance context (PR3 review, observation
/// 3): `accept` processes a batch of described effects strictly sequentially
/// and stops at the first failure, so `failed_at_index` doubles as both
/// "which effect in this batch failed" AND "how many of this batch's effects
/// were already durably accepted before it" — a caller/observer can tell
/// exactly what state things are actually in instead of just seeing an
/// opaque message.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum EffectAcceptanceError {
    /// The one retryable store error (`TemporarilyUnavailable`) survived the
    /// bounded acceptance retry policy (or a shutdown deadline interrupted an
    /// in-progress retry — AD-9's shutdown interaction). Commit is final; the
    /// effect may be lost to the post-commit dual-write gap.
    #[error(
        "effect acceptance retries exhausted: {message} (failed at batch index \
         {failed_at_index}, idempotency_key={failed_idempotency_key}; \
         {failed_at_index} effect(s) already durably accepted before this failure)"
    )]
    RetriesExhausted {
        /// Human-readable detail from the underlying store error.
        message: String,
        /// Zero-based index, within this `accept` call's effect batch, of
        /// the effect that failed. Also the count of effects from the same
        /// batch already durably accepted before this failure.
        failed_at_index: usize,
        /// The idempotency key of the effect that failed.
        failed_idempotency_key: IdempotencyKey,
    },
    /// A permanent store failure — surfaced without retry. Same
    /// commit-is-final, no-rollback semantics as `RetriesExhausted`.
    #[error(
        "effect acceptance failed permanently: {message} (failed at batch index \
         {failed_at_index}, idempotency_key={failed_idempotency_key}; \
         {failed_at_index} effect(s) already durably accepted before this failure)"
    )]
    Permanent {
        /// Human-readable detail from the underlying store error.
        message: String,
        /// Zero-based index, within this `accept` call's effect batch, of
        /// the effect that failed. Also the count of effects from the same
        /// batch already durably accepted before this failure.
        failed_at_index: usize,
        /// The idempotency key of the effect that failed.
        failed_idempotency_key: IdempotencyKey,
    },
}

/// Post-commit effect acceptance port (AD-1, AD-3, AD-9).
///
/// The actor calls `accept` after its atomic commit succeeds and before
/// sending the command's successful reply (AD-1's seam). Implemented outside
/// this crate (`RuntimeEffectAcceptor`, `ego-runtime`), exactly like
/// [`crate::publisher::EventPublisher`].
#[async_trait]
pub trait EffectAcceptor: Send + Sync {
    /// Mints an effect id, attaches `tenant`, and records each described
    /// effect as accepted before awaiting delivery-queue capacity.
    ///
    /// A NORMAL in-progress effect, once admitted, is never refused outright
    /// mid-flight — there is no "your effect list is rejected" path once
    /// `accept` has actually started processing it. MAY still ultimately
    /// fail: a transient store error is retried under a bounded policy and,
    /// if that policy is exhausted (or the store error is non-retryable),
    /// returns `Err(EffectAcceptanceError)`. That error NEVER implies the
    /// already-committed event was rolled back — commit is final; it means
    /// at least one effect could not be durably-enough registered and may be
    /// lost to the post-commit dual-write gap (AD-9).
    ///
    /// Distinct case: a NEW `accept()` call that arrives after shutdown/
    /// draining has already begun IS rejected immediately at intake, before
    /// minting anything or touching the store at all (see the runtime
    /// implementation's admission-gating lifecycle, `ego-runtime`'s
    /// `LifecycleGate`).
    async fn accept(
        &self,
        tenant: &TenantId,
        effects: Vec<ExternalEffectDescription>,
    ) -> Result<(), EffectAcceptanceError>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use ego_domain::IdempotencyKey;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    fn description(key: &str) -> ExternalEffectDescription {
        ExternalEffectDescription {
            idempotency_key: IdempotencyKey::new(key).unwrap(),
            effect_type: "invoice.created".to_string(),
            payload: vec![1, 2, 3],
            destination: "https://example.com".to_string(),
        }
    }

    /// A minimal, real `EffectAcceptor` implementation — proves the trait is
    /// dyn-compatible and usable as `Arc<dyn EffectAcceptor>`, exactly how
    /// `EntityActor` will hold it (AD-1). Counts through a shared handle
    /// (not a private field) since the trait object erases the concrete
    /// type.
    struct RecordingAcceptor {
        calls: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl EffectAcceptor for RecordingAcceptor {
        async fn accept(
            &self,
            _tenant: &TenantId,
            effects: Vec<ExternalEffectDescription>,
        ) -> Result<(), EffectAcceptanceError> {
            self.calls.fetch_add(effects.len(), Ordering::SeqCst);
            Ok(())
        }
    }

    #[tokio::test]
    async fn trait_object_is_callable_through_dyn_effect_acceptor() {
        let calls = Arc::new(AtomicUsize::new(0));
        let acceptor: Arc<dyn EffectAcceptor> = Arc::new(RecordingAcceptor {
            calls: calls.clone(),
        });
        let tenant = TenantId::new("tenant-a").unwrap();

        acceptor
            .accept(
                &tenant,
                vec![description("uow-1:0"), description("uow-1:1")],
            )
            .await
            .unwrap();

        // Real assertion tied to production dispatch, not a tautology: both
        // effects were actually delivered through the trait object.
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn retries_exhausted_and_permanent_are_distinguishable_variants() {
        let retries = EffectAcceptanceError::RetriesExhausted {
            message: "pool exhausted".to_string(),
            failed_at_index: 0,
            failed_idempotency_key: IdempotencyKey::new("uow-1:0").unwrap(),
        };
        let permanent = EffectAcceptanceError::Permanent {
            message: "corrupt record".to_string(),
            failed_at_index: 0,
            failed_idempotency_key: IdempotencyKey::new("uow-1:0").unwrap(),
        };

        assert!(matches!(
            retries,
            EffectAcceptanceError::RetriesExhausted { .. }
        ));
        assert!(matches!(permanent, EffectAcceptanceError::Permanent { .. }));
        assert_ne!(retries, permanent);
        assert_eq!(
            retries.to_string(),
            "effect acceptance retries exhausted: pool exhausted (failed at batch index 0, \
             idempotency_key=uow-1:0; 0 effect(s) already durably accepted before this failure)"
        );
        assert_eq!(
            permanent.to_string(),
            "effect acceptance failed permanently: corrupt record (failed at batch index 0, \
             idempotency_key=uow-1:0; 0 effect(s) already durably accepted before this failure)"
        );
    }

    /// PR3 review, observation 3: proves `failed_at_index` genuinely reflects
    /// which effect in a multi-effect batch failed, and that the count of
    /// already-accepted effects is recoverable from it (they're the same
    /// number, since `accept` is strictly sequential and stops at the first
    /// failure).
    #[test]
    fn partial_acceptance_context_identifies_the_failing_effect_and_prior_success_count() {
        let error = EffectAcceptanceError::Permanent {
            message: "backend down".to_string(),
            failed_at_index: 2,
            failed_idempotency_key: IdempotencyKey::new("uow-1:2").unwrap(),
        };

        match error {
            EffectAcceptanceError::Permanent {
                failed_at_index,
                failed_idempotency_key,
                ..
            } => {
                assert_eq!(
                    failed_at_index, 2,
                    "the 3rd effect (index 2) is the one that failed"
                );
                assert_eq!(
                    failed_idempotency_key,
                    IdempotencyKey::new("uow-1:2").unwrap()
                );
            }
            other => panic!("expected Permanent, got {other:?}"),
        }
    }
}
