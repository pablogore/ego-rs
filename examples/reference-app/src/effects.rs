//! `WelcomeEmailExecutor` — PROD-002 Phase 8 dogfood: a trivial, log-only
//! `ExternalEffectExecutor` for `UserEntity`'s "welcome email" effect
//! (`domain/user.rs::WELCOME_EMAIL_EFFECT_TYPE`).
//!
//! Never a real mailer/HTTP call — same "no external client dependency"
//! posture the dogfood provider in `providers::pricing_lookup` already
//! takes for `ExternalDataProvider`. Always reports [`AttemptOutcome::Success`]
//! after logging the effect's destination and idempotency key, which is
//! enough to prove real delivery happened through the real
//! `DeliveryRunner`, without depending on an actual mailer.

use async_trait::async_trait;
use ego_domain::ExternalEffectDescription;
use ego_runtime::effects::{AttemptOutcome, EffectContext, ExternalEffectExecutor};

/// Logs every delivery attempt (via `println!`, matching the rest of this
/// crate — no `tracing`/`log` dependency exists here to reach for instead)
/// and always succeeds.
pub struct WelcomeEmailExecutor;

#[async_trait]
impl ExternalEffectExecutor for WelcomeEmailExecutor {
    async fn execute(
        &self,
        effect: &ExternalEffectDescription,
        ctx: &EffectContext,
    ) -> AttemptOutcome {
        println!(
            "reference-app: welcome email delivered (destination={}, idempotency_key={}, attempt={})",
            effect.destination, ctx.idempotency_key, ctx.attempt
        );
        AttemptOutcome::Success
    }
}
