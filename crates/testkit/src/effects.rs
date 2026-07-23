//! Recording `ExternalEffectExecutor` test double (CORE-019 Phase 12.1).
//!
//! Records every delivery attempt (`effect_type`, `destination`, `payload`,
//! attempt number) so a test can assert on delivery/retry/dedup behavior
//! without standing up a real external system.

use std::sync::Mutex;

use async_trait::async_trait;
use ego_domain::ExternalEffectDescription;
use ego_runtime::effects::{AttemptOutcome, EffectContext, ExternalEffectExecutor};

/// One recorded delivery attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordedAttempt {
    /// The `effect_type` of the effect this attempt was for.
    pub effect_type: String,
    /// The effect's destination, passed through unexamined.
    pub destination: String,
    /// The effect's payload, passed through unexamined.
    pub payload: Vec<u8>,
    /// The 1-based attempt number for this dispatch.
    pub attempt: u32,
}

/// Records every attempt an [`ExternalEffectExecutor`] receives — same-contract
/// principle: a real implementation of the real production trait, not a
/// look-alike.
///
/// Configure a scripted outcome sequence via [`RecordingExecutor::with_outcomes`]
/// to exercise retry (e.g. a `RetryableFailure` followed by a `Success`); the
/// final scripted outcome repeats once the sequence is exhausted, so a real
/// delivery runner attempting more times than were explicitly scripted never
/// panics.
pub struct RecordingExecutor {
    attempts: Mutex<Vec<RecordedAttempt>>,
    outcomes: Vec<AttemptOutcome>,
}

impl RecordingExecutor {
    /// An executor that always succeeds, on any attempt.
    pub fn always_succeeds() -> Self {
        Self::with_outcomes(vec![AttemptOutcome::Success])
    }

    /// An executor that replays `outcomes` in order (indexed by the 1-based
    /// attempt number), repeating the final entry once exhausted.
    ///
    /// # Panics
    /// Panics if `outcomes` is empty — there would be nothing to return.
    pub fn with_outcomes(outcomes: Vec<AttemptOutcome>) -> Self {
        assert!(
            !outcomes.is_empty(),
            "RecordingExecutor::with_outcomes needs at least one scripted outcome"
        );
        Self {
            attempts: Mutex::new(Vec::new()),
            outcomes,
        }
    }

    /// Every attempt recorded so far, in delivery order.
    pub fn attempts(&self) -> Vec<RecordedAttempt> {
        self.attempts.lock().unwrap().clone()
    }
}

#[async_trait]
impl ExternalEffectExecutor for RecordingExecutor {
    async fn execute(
        &self,
        effect: &ExternalEffectDescription,
        ctx: &EffectContext,
    ) -> AttemptOutcome {
        self.attempts.lock().unwrap().push(RecordedAttempt {
            effect_type: effect.effect_type.clone(),
            destination: effect.destination.clone(),
            payload: effect.payload.clone(),
            attempt: ctx.attempt,
        });

        let idx = (ctx.attempt as usize).saturating_sub(1);
        self.outcomes
            .get(idx)
            .or_else(|| self.outcomes.last())
            .cloned()
            .expect("checked non-empty at construction")
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use ego_domain::{ExternalEffectDescription, IdempotencyKey, TenantId};
    use ego_runtime::effects::{AttemptOutcome, EffectContext, EffectId, ExternalEffectExecutor};

    use super::RecordingExecutor;

    fn description(effect_type: &str, key: &str) -> ExternalEffectDescription {
        ExternalEffectDescription {
            idempotency_key: IdempotencyKey::new(key).unwrap(),
            effect_type: effect_type.to_string(),
            payload: vec![1, 2, 3],
            destination: "https://example.com/probe".to_string(),
        }
    }

    fn ctx(attempt: u32) -> EffectContext {
        EffectContext {
            effect_id: EffectId::new(),
            tenant: TenantId::new("tenant-a").unwrap(),
            attempt,
            idempotency_key: IdempotencyKey::new("uow-1:0").unwrap(),
        }
    }

    #[tokio::test]
    async fn records_effect_type_destination_payload_and_attempt() {
        let executor = RecordingExecutor::always_succeeds();

        let outcome = executor
            .execute(&description("invoice.created", "uow-1:0"), &ctx(1))
            .await;

        assert_eq!(outcome, AttemptOutcome::Success);
        let attempts = executor.attempts();
        assert_eq!(attempts.len(), 1, "exactly one attempt must be recorded");
        assert_eq!(attempts[0].effect_type, "invoice.created");
        assert_eq!(attempts[0].destination, "https://example.com/probe");
        assert_eq!(attempts[0].payload, vec![1, 2, 3]);
        assert_eq!(attempts[0].attempt, 1);
    }

    #[tokio::test]
    async fn scripted_outcomes_support_retry_then_success_and_repeat_the_last_entry() {
        let executor = Arc::new(RecordingExecutor::with_outcomes(vec![
            AttemptOutcome::RetryableFailure("timeout".to_string()),
            AttemptOutcome::Success,
        ]));

        let first = executor
            .execute(&description("invoice.created", "uow-2:0"), &ctx(1))
            .await;
        let second = executor
            .execute(&description("invoice.created", "uow-2:0"), &ctx(2))
            .await;
        // A 3rd attempt beyond the scripted sequence repeats the last entry
        // rather than panicking — real delivery runners may attempt more
        // times than were explicitly scripted.
        let third = executor
            .execute(&description("invoice.created", "uow-2:0"), &ctx(3))
            .await;

        assert_eq!(
            first,
            AttemptOutcome::RetryableFailure("timeout".to_string())
        );
        assert_eq!(second, AttemptOutcome::Success);
        assert_eq!(third, AttemptOutcome::Success);
        assert_eq!(
            executor.attempts().len(),
            3,
            "every attempt is recorded, including the repeated one"
        );
    }
}
