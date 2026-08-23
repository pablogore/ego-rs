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
    async fn execute(
        &self,
        effect: &ExternalEffectDescription,
        ctx: &EffectContext,
    ) -> AttemptOutcome;
}
