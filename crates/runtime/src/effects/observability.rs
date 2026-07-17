//! Observability signals (CORE-019 Phase 11, spec: "Observability Signals").
//!
//! Mirrors `ego-scheduler`'s `metric.rs` convention (`crates/ego-scheduler/src/metric.rs`):
//! named `log_*` functions wrapping `tracing` macros, called from the actual
//! delivery logic rather than inlining `tracing::info!`/`warn!` at every call
//! site. Every signal here carries only the runtime effect identifier,
//! `effect_type`, `destination`, tenant, and a redacted/hashed idempotency
//! key — `payload` MUST NOT appear in any signal by default (spec: "Payload
//! never appears in a default signal").
//!
//! Signal set (spec: "Observability Signals", design.md §9): `accepted`,
//! `dispatch_started`, `attempt`, `success`, `retry_scheduled`,
//! `terminal_failed`, `deduplicated`, `executor_missing`, `queue_depth`,
//! `oldest_pending_age`, `drain_incomplete`.

use std::time::Duration;

use sha2::{Digest, Sha256};
use tracing::{info, warn};

use super::store::AcceptedEffect;

/// Redacts an idempotency key to a short, deterministic, non-reversible hash
/// — never the raw key — for cross-log correlation (spec: "Observability
/// Signals": "a redacted/hashed idempotency key"). Deterministic (unlike
/// `std::collections::hash_map::DefaultHasher`, which is randomly seeded per
/// process) so the same key correlates across process restarts.
pub(crate) fn hashed_key(key: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(key.as_bytes());
    let digest = hasher.finalize();
    // First 4 bytes (8 hex chars) is plenty for log correlation — this is a
    // redaction aid, not a security boundary, so the full 32-byte digest
    // would be needless log noise.
    digest[..4].iter().map(|b| format!("{b:02x}")).collect()
}

/// `accepted`: the effect was recorded by the runtime after its command's
/// commit succeeded, before it ever reaches the admission queue.
pub(crate) fn log_accepted(effect: &AcceptedEffect) {
    info!(
        effect_id = %effect.id,
        effect_type = %effect.description.effect_type,
        destination = %effect.description.destination,
        tenant = %effect.tenant.as_str(),
        idempotency_key_hash = %hashed_key(effect.description.idempotency_key.as_str()),
        "accepted"
    );
}

/// `dispatch_started`: the delivery runner has begun processing one accepted
/// effect (design.md §9: "runner pre-execute").
pub(crate) fn log_dispatch_started(effect: &AcceptedEffect) {
    info!(
        effect_id = %effect.id,
        effect_type = %effect.description.effect_type,
        destination = %effect.description.destination,
        tenant = %effect.tenant.as_str(),
        idempotency_key_hash = %hashed_key(effect.description.idempotency_key.as_str()),
        "dispatch_started"
    );
}

/// `attempt`: one executor invocation is about to run; `attempt` is the
/// 1-based attempt number handed to the executor's `EffectContext`.
pub(crate) fn log_attempt(effect: &AcceptedEffect, attempt: u32) {
    info!(
        effect_id = %effect.id,
        effect_type = %effect.description.effect_type,
        destination = %effect.description.destination,
        tenant = %effect.tenant.as_str(),
        idempotency_key_hash = %hashed_key(effect.description.idempotency_key.as_str()),
        attempt,
        "attempt"
    );
}

/// `success`: the executor reported `AttemptOutcome::Success` for this
/// attempt.
pub(crate) fn log_success(effect: &AcceptedEffect) {
    info!(
        effect_id = %effect.id,
        effect_type = %effect.description.effect_type,
        destination = %effect.description.destination,
        tenant = %effect.tenant.as_str(),
        idempotency_key_hash = %hashed_key(effect.description.idempotency_key.as_str()),
        "success"
    );
}

/// `retry_scheduled`: a `RetryableFailure` is being re-enqueued after
/// `backoff`, as attempt `next_attempt`.
pub(crate) fn log_retry_scheduled(effect: &AcceptedEffect, next_attempt: u32, backoff: Duration) {
    info!(
        effect_id = %effect.id,
        effect_type = %effect.description.effect_type,
        destination = %effect.description.destination,
        tenant = %effect.tenant.as_str(),
        idempotency_key_hash = %hashed_key(effect.description.idempotency_key.as_str()),
        next_attempt,
        backoff_ms = backoff.as_millis() as u64,
        "retry_scheduled"
    );
}

/// `terminal_failed`: the effect will never be retried again; `reason` is a
/// short, human-readable explanation — never the payload.
pub(crate) fn log_terminal_failed(effect: &AcceptedEffect, reason: &str) {
    warn!(
        effect_id = %effect.id,
        effect_type = %effect.description.effect_type,
        destination = %effect.description.destination,
        tenant = %effect.tenant.as_str(),
        idempotency_key_hash = %hashed_key(effect.description.idempotency_key.as_str()),
        reason,
        "terminal_failed"
    );
}

/// `deduplicated`: the scoped idempotency key was already reserved with an
/// identical fingerprint — this attempt is a logical duplicate.
pub(crate) fn log_deduplicated(effect: &AcceptedEffect) {
    info!(
        effect_id = %effect.id,
        effect_type = %effect.description.effect_type,
        destination = %effect.description.destination,
        tenant = %effect.tenant.as_str(),
        idempotency_key_hash = %hashed_key(effect.description.idempotency_key.as_str()),
        "deduplicated"
    );
}

/// `executor_missing`: no `ExternalEffectExecutor` is registered for this
/// effect's `effect_type` — fail-closed, loud (spec: "Backward
/// Compatibility").
pub(crate) fn log_executor_missing(effect: &AcceptedEffect) {
    warn!(
        effect_id = %effect.id,
        effect_type = %effect.description.effect_type,
        destination = %effect.description.destination,
        tenant = %effect.tenant.as_str(),
        idempotency_key_hash = %hashed_key(effect.description.idempotency_key.as_str()),
        "executor_missing"
    );
}

/// `queue_depth`: how many accepted effects currently sit in the bounded
/// admission queue.
pub(crate) fn log_queue_depth(depth: usize) {
    info!(queue_depth = depth, "queue_depth");
}

/// `oldest_pending_age`: how long the oldest still-queued effect has been
/// waiting; `None` when nothing is queued.
pub(crate) fn log_oldest_pending_age(age: Option<Duration>) {
    info!(
        oldest_pending_age_ms = age.map(|a| a.as_millis() as u64).unwrap_or(0),
        "oldest_pending_age"
    );
}

/// `drain_incomplete`: shutdown's drain deadline passed with `recovered`
/// effect(s) still in flight, recovered back to `Pending` rather than lost
/// silently (spec: "Shutdown drains within deadline or signals
/// incompleteness").
///
/// CORE-019 rebase reconciliation note: this signal's only production call
/// site was `RuntimeEffectAcceptor::drain` — removed when PR3's review
/// rounds replaced it with `EffectRuntimeHandle::shutdown_and_wait`, which
/// reports an honest `Result` but no longer carries a recovered-effect
/// *count*. `service-sdk`'s `builder.rs` teardown hook now maps that
/// `Result`'s `Err` to a `RuntimeInfraError::Teardown` (still surfaced, never
/// silently discarded), but does not call this fn — flagged for the
/// maintainer to decide whether restoring a countable drain-incomplete
/// signal is worth threading a count back through `shutdown_and_wait`.
#[allow(dead_code)]
pub(crate) fn log_drain_incomplete(recovered: u64) {
    warn!(recovered_effect_count = recovered, "drain_incomplete");
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::effects::store::EffectId;
    use ego_domain::{ExternalEffectDescription, IdempotencyKey, TenantId};
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    #[derive(Default, Clone, Debug)]
    struct CapturedEvent {
        fields: HashMap<String, String>,
    }

    struct FieldRecorder<'a>(&'a mut HashMap<String, String>);

    impl tracing::field::Visit for FieldRecorder<'_> {
        fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
            self.0.insert(field.name().to_string(), format!("{value:?}"));
        }
        fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
            self.0.insert(field.name().to_string(), value.to_string());
        }
    }

    struct TestSubscriber {
        events: Arc<Mutex<Vec<CapturedEvent>>>,
    }

    impl tracing::Subscriber for TestSubscriber {
        fn enabled(&self, _metadata: &tracing::Metadata<'_>) -> bool {
            true
        }
        fn new_span(&self, _span: &tracing::span::Attributes<'_>) -> tracing::span::Id {
            tracing::span::Id::from_u64(1)
        }
        fn record(&self, _span: &tracing::span::Id, _values: &tracing::span::Record<'_>) {}
        fn record_follows_from(&self, _span: &tracing::span::Id, _follows: &tracing::span::Id) {}
        fn event(&self, event: &tracing::Event<'_>) {
            let mut fields = HashMap::new();
            event.record(&mut FieldRecorder(&mut fields));
            self.events.lock().unwrap().push(CapturedEvent { fields });
        }
        fn enter(&self, _span: &tracing::span::Id) {}
        fn exit(&self, _span: &tracing::span::Id) {}
    }

    /// Installs one real, always-enabled subscriber as `tracing`'s *global*
    /// default (CORE-027 flaky-triage root cause fix).
    ///
    /// A `log_*` callsite's very first hit anywhere in the process resolves
    /// "is anyone interested?" against whatever the current default is on
    /// the calling thread. Before this fix, that could be `tracing-core`'s
    /// built-in no-op default — which answers "never" — if the first thread
    /// to reach a given callsite happened to be one that never installed a
    /// subscriber itself (true for `effects::runner`'s and
    /// `effects::acceptor`'s own tests, which call these same production
    /// `log_*` functions directly). That "never" verdict then caches
    /// permanently for that callsite, so a later `capture_events` call can
    /// silently miss it even with its own subscriber active — a race that a
    /// per-file mutex around our own `with_default` calls cannot reach,
    /// since the other side of the race is unrelated test code with no
    /// subscriber at all.
    ///
    /// Fix: use `tracing::subscriber::set_global_default` (the public,
    /// documented mechanism for "the process always has a default
    /// subscriber") to replace the no-op built-in default with a real one
    /// whose `enabled()` always returns `true`, exactly once, before any
    /// test runs. A callsite's first hit then always resolves to "someone's
    /// interested", so `event()` always fires — delivered to this harmless
    /// global default when no test-local subscriber is active, or correctly
    /// overridden by a thread's own `with_default` during a capture window
    /// (thread-local always wins over the global default on that thread).
    pub(crate) fn ensure_interest_cache_race_immune() {
        static ONCE: std::sync::Once = std::sync::Once::new();
        ONCE.call_once(|| {
            struct AlwaysOn;
            impl tracing::Subscriber for AlwaysOn {
                fn enabled(&self, _metadata: &tracing::Metadata<'_>) -> bool {
                    true
                }
                fn new_span(&self, _span: &tracing::span::Attributes<'_>) -> tracing::span::Id {
                    tracing::span::Id::from_u64(1)
                }
                fn record(&self, _span: &tracing::span::Id, _values: &tracing::span::Record<'_>) {}
                fn record_follows_from(&self, _span: &tracing::span::Id, _follows: &tracing::span::Id) {}
                fn event(&self, _event: &tracing::Event<'_>) {}
                fn enter(&self, _span: &tracing::span::Id) {}
                fn exit(&self, _span: &tracing::span::Id) {}
            }
            // Ignore the error: if something else already set a global
            // default first, that default is at least as real as ours.
            let _ = tracing::subscriber::set_global_default(AlwaysOn);
        });
    }

    /// Serializes every `capture_events` call in the crate against the
    /// documented `Arc::try_unwrap` transient-strong-count hazard (see
    /// `providers::access`'s `capture_events`, which shares this exact
    /// static via `crate::effects::observability::tests::
    /// CAPTURE_EVENTS_GUARD`). Belt-and-braces alongside
    /// `ensure_interest_cache_race_immune` above, not a substitute for it.
    pub(crate) static CAPTURE_EVENTS_GUARD: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// Runs `f` under a subscriber that records every emitted event's fields,
    /// returning them all — lets a test assert on the exact fields a `log_*`
    /// call actually produced, not just that it compiles.
    fn capture_events(f: impl FnOnce()) -> Vec<CapturedEvent> {
        ensure_interest_cache_race_immune();
        let _guard = CAPTURE_EVENTS_GUARD
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let events = Arc::new(Mutex::new(Vec::new()));
        let subscriber = TestSubscriber {
            events: events.clone(),
        };
        tracing::subscriber::with_default(subscriber, f);
        Arc::try_unwrap(events).unwrap().into_inner().unwrap()
    }

    fn find_message<'a>(events: &'a [CapturedEvent], message: &str) -> &'a CapturedEvent {
        events
            .iter()
            .find(|e| e.fields.get("message").map(|m| m.as_str()) == Some(message))
            .unwrap_or_else(|| panic!("no captured event with message {message:?}"))
    }

    const DISTINCTIVE_PAYLOAD_MARKER: &str = "PAYLOAD-MUST-NEVER-LEAK-3f9a";

    fn sample_effect() -> AcceptedEffect {
        AcceptedEffect {
            id: EffectId::new(),
            tenant: TenantId::new("tenant-a").unwrap(),
            attempt: 0,
            description: Arc::new(ExternalEffectDescription {
                idempotency_key: IdempotencyKey::new("uow-1:0").unwrap(),
                effect_type: "invoice.created".to_string(),
                payload: DISTINCTIVE_PAYLOAD_MARKER.as_bytes().to_vec(),
                destination: "https://example.com".to_string(),
            }),
        }
    }

    #[test]
    fn hashed_key_is_deterministic_and_never_equals_the_raw_key() {
        let a = hashed_key("uow-1:0");
        let b = hashed_key("uow-1:0");
        assert_eq!(a, b, "the same key must hash to the same redacted value");
        assert_ne!(a, "uow-1:0");
    }

    #[test]
    fn hashed_key_differs_for_different_keys() {
        assert_ne!(hashed_key("uow-1:0"), hashed_key("uow-1:1"));
    }

    #[test]
    fn accepted_signal_carries_required_correlation_fields() {
        let effect = sample_effect();
        let events = capture_events(|| log_accepted(&effect));

        let event = find_message(&events, "accepted");
        assert_eq!(event.fields.get("effect_id").map(String::as_str), Some(effect.id.to_string()).as_deref());
        assert_eq!(event.fields.get("effect_type").map(String::as_str), Some("invoice.created"));
        assert_eq!(event.fields.get("destination").map(String::as_str), Some("https://example.com"));
        assert_eq!(event.fields.get("tenant").map(String::as_str), Some("tenant-a"));
        assert!(event.fields.contains_key("idempotency_key_hash"));
    }

    #[test]
    fn every_signal_redacts_the_idempotency_key_and_never_carries_the_raw_key_or_payload() {
        let effect = sample_effect();
        let events = capture_events(|| {
            log_accepted(&effect);
            log_dispatch_started(&effect);
            log_attempt(&effect, 1);
            log_success(&effect);
            log_retry_scheduled(&effect, 2, Duration::from_millis(100));
            log_terminal_failed(&effect, "attempt cap exceeded");
            log_deduplicated(&effect);
            log_executor_missing(&effect);
            log_queue_depth(3);
            log_oldest_pending_age(Some(Duration::from_secs(1)));
            log_drain_incomplete(1);
        });

        assert_eq!(events.len(), 11, "every log_* call must emit exactly one event");

        for event in &events {
            for (field, value) in &event.fields {
                assert!(
                    !value.contains(DISTINCTIVE_PAYLOAD_MARKER),
                    "field {field:?} leaked the payload: {value:?}"
                );
                assert!(
                    field != "idempotency_key_hash" || value != "uow-1:0",
                    "idempotency_key_hash must never equal the raw key"
                );
                assert_ne!(
                    value, "uow-1:0",
                    "field {field:?} must never carry the raw idempotency key verbatim"
                );
            }
        }

        // Every per-effect signal carries the id/effect_type/destination/
        // tenant/hashed-key correlation set (spec: "Observability Signals").
        for message in [
            "accepted",
            "dispatch_started",
            "attempt",
            "success",
            "retry_scheduled",
            "terminal_failed",
            "deduplicated",
            "executor_missing",
        ] {
            let event = find_message(&events, message);
            for required in [
                "effect_id",
                "effect_type",
                "destination",
                "tenant",
                "idempotency_key_hash",
            ] {
                assert!(
                    event.fields.contains_key(required),
                    "{message} signal is missing required field {required}"
                );
            }
        }
    }
}
