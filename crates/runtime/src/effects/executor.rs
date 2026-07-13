//! Executor trait and per-attempt context (CORE-019 Phase 2).
//!
//! Exactly one attempt of one effect per [`ExternalEffectExecutor::execute`]
//! call — no retry/backoff/dedup/persistence here; that's the delivery
//! runner's job (Phase 6, out of scope for this slice).

use async_trait::async_trait;
use ego_domain::{ExternalEffectDescription, IdempotencyKey, TenantId};

use super::store::EffectId;

/// The outcome of one delivery attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AttemptOutcome {
    /// The effect was delivered successfully.
    Success,
    /// The attempt failed but may be retried (subject to the attempt cap).
    RetryableFailure(String),
    /// The attempt failed and must never be retried.
    TerminalFailure(String),
}

/// Read-only per-attempt facts handed to an [`ExternalEffectExecutor`].
#[derive(Debug, Clone)]
pub struct EffectContext {
    /// The runtime-minted effect identifier.
    pub effect_id: EffectId,
    /// The tenant established at acceptance time — a fact the executor
    /// cannot substitute or mint (spec: "Tenant Isolation").
    pub tenant: TenantId,
    /// The 1-based attempt number for this dispatch.
    pub attempt: u32,
    /// The idempotency key that MUST be propagated to the destination.
    pub idempotency_key: IdempotencyKey,
}

/// One owner per `effect_type`, transport-agnostic (spec: "ExternalEffectExecutor
/// Registry — One Owner Per Type").
#[async_trait]
pub trait ExternalEffectExecutor: Send + Sync {
    /// Executes exactly one attempt of `effect`.
    async fn execute(&self, effect: &ExternalEffectDescription, ctx: &EffectContext) -> AttemptOutcome;

    /// Doc-only signal for end-to-end semantics; does not change runtime
    /// behavior. Defaults to `false` — most executors don't cooperate on
    /// idempotency and callers must not assume they do.
    fn honors_idempotency_key(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct AlwaysSucceeds;

    #[async_trait]
    impl ExternalEffectExecutor for AlwaysSucceeds {
        async fn execute(&self, _effect: &ExternalEffectDescription, _ctx: &EffectContext) -> AttemptOutcome {
            AttemptOutcome::Success
        }
    }

    struct IdempotentExecutor;

    #[async_trait]
    impl ExternalEffectExecutor for IdempotentExecutor {
        async fn execute(&self, _effect: &ExternalEffectDescription, _ctx: &EffectContext) -> AttemptOutcome {
            AttemptOutcome::RetryableFailure("timeout".into())
        }

        fn honors_idempotency_key(&self) -> bool {
            true
        }
    }

    fn ctx() -> EffectContext {
        EffectContext {
            effect_id: EffectId::new(),
            tenant: TenantId::new("tenant-a").unwrap(),
            attempt: 1,
            idempotency_key: IdempotencyKey::new("uow-1:0").unwrap(),
        }
    }

    fn description() -> ExternalEffectDescription {
        ExternalEffectDescription {
            idempotency_key: IdempotencyKey::new("uow-1:0").unwrap(),
            effect_type: "invoice.created".to_string(),
            payload: vec![],
            destination: "https://example.com".to_string(),
        }
    }

    #[tokio::test]
    async fn default_honors_idempotency_key_is_false() {
        let executor = AlwaysSucceeds;
        assert!(!executor.honors_idempotency_key());
        assert_eq!(
            executor.execute(&description(), &ctx()).await,
            AttemptOutcome::Success
        );
    }

    #[tokio::test]
    async fn executor_may_override_honors_idempotency_key() {
        let executor = IdempotentExecutor;
        assert!(executor.honors_idempotency_key());
        assert_eq!(
            executor.execute(&description(), &ctx()).await,
            AttemptOutcome::RetryableFailure("timeout".into())
        );
    }
}
