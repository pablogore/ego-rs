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
//!
//! CORE-027 flaky-triage note: the per-effect signals' field construction and
//! redaction (`effect_fields`) is a pure, deterministic function, kept
//! separate from the `tracing::info!`/`warn!` calls that emit it. This lets
//! the correctness bar (redaction, no payload leak, correct values) be
//! asserted directly, without depending on `tracing-core`'s process-wide
//! per-callsite interest cache — which under a full-crate parallel test
//! sweep can race against unrelated tests exercising these same production
//! callsites with no subscriber installed, silently dropping a captured
//! event. The `log_*` functions' `tracing::info!`/`warn!` calls remain
//! compile-time-checked wiring only (the macro fixes field names/values at
//! the call site); no test captures through `tracing`'s dispatch machinery
//! anymore, since that capturing test was itself shown to still flake under
//! the same race and added no coverage beyond what the macro already
//! guarantees at compile time.

use std::time::Duration;

use sha2::{Digest, Sha256};
use tracing::{info, warn};
use uuid::Uuid;

use super::store::{AcceptedEffect, EffectId, Timestamp};

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

/// The correlation fields shared by every per-effect signal (spec:
/// "Observability Signals"). Computed once, deterministically, so redaction
/// and field-shape can be asserted directly in tests without emitting
/// through `tracing` at all.
struct EffectFields {
    effect_id: String,
    effect_type: String,
    destination: String,
    tenant: String,
    idempotency_key_hash: String,
}

fn effect_fields(effect: &AcceptedEffect) -> EffectFields {
    EffectFields {
        effect_id: effect.id.to_string(),
        effect_type: effect.description.effect_type.clone(),
        destination: effect.description.destination.clone(),
        tenant: effect.tenant.as_str().to_string(),
        idempotency_key_hash: hashed_key(effect.description.idempotency_key.as_str()),
    }
}

/// `accepted`: the effect was recorded by the runtime after its command's
/// commit succeeded, before it ever reaches the admission queue.
pub(crate) fn log_accepted(effect: &AcceptedEffect) {
    let f = effect_fields(effect);
    info!(
        effect_id = %f.effect_id,
        effect_type = %f.effect_type,
        destination = %f.destination,
        tenant = %f.tenant,
        idempotency_key_hash = %f.idempotency_key_hash,
        "accepted"
    );
}

/// `dispatch_started`: the delivery runner has begun processing one accepted
/// effect (design.md §9: "runner pre-execute").
pub(crate) fn log_dispatch_started(effect: &AcceptedEffect) {
    let f = effect_fields(effect);
    info!(
        effect_id = %f.effect_id,
        effect_type = %f.effect_type,
        destination = %f.destination,
        tenant = %f.tenant,
        idempotency_key_hash = %f.idempotency_key_hash,
        "dispatch_started"
    );
}

/// `attempt`: one executor invocation is about to run; `attempt` is the
/// 1-based attempt number handed to the executor's `EffectContext`.
pub(crate) fn log_attempt(effect: &AcceptedEffect, attempt: u32) {
    let f = effect_fields(effect);
    info!(
        effect_id = %f.effect_id,
        effect_type = %f.effect_type,
        destination = %f.destination,
        tenant = %f.tenant,
        idempotency_key_hash = %f.idempotency_key_hash,
        attempt,
        "attempt"
    );
}

/// `success`: the executor reported `AttemptOutcome::Success` for this
/// attempt.
pub(crate) fn log_success(effect: &AcceptedEffect) {
    let f = effect_fields(effect);
    info!(
        effect_id = %f.effect_id,
        effect_type = %f.effect_type,
        destination = %f.destination,
        tenant = %f.tenant,
        idempotency_key_hash = %f.idempotency_key_hash,
        "success"
    );
}

/// `retry_scheduled`: a `RetryableFailure` is being re-enqueued after
/// `backoff`, as attempt `next_attempt`.
pub(crate) fn log_retry_scheduled(effect: &AcceptedEffect, next_attempt: u32, backoff: Duration) {
    let f = effect_fields(effect);
    info!(
        effect_id = %f.effect_id,
        effect_type = %f.effect_type,
        destination = %f.destination,
        tenant = %f.tenant,
        idempotency_key_hash = %f.idempotency_key_hash,
        next_attempt,
        backoff_ms = backoff.as_millis() as u64,
        "retry_scheduled"
    );
}

/// `terminal_failed`: the effect will never be retried again; `reason` is a
/// short, human-readable explanation — never the payload.
pub(crate) fn log_terminal_failed(effect: &AcceptedEffect, reason: &str) {
    let f = effect_fields(effect);
    warn!(
        effect_id = %f.effect_id,
        effect_type = %f.effect_type,
        destination = %f.destination,
        tenant = %f.tenant,
        idempotency_key_hash = %f.idempotency_key_hash,
        reason,
        "terminal_failed"
    );
}

/// `deduplicated`: the scoped idempotency key was already reserved with an
/// identical fingerprint — this attempt is a logical duplicate.
pub(crate) fn log_deduplicated(effect: &AcceptedEffect) {
    let f = effect_fields(effect);
    info!(
        effect_id = %f.effect_id,
        effect_type = %f.effect_type,
        destination = %f.destination,
        tenant = %f.tenant,
        idempotency_key_hash = %f.idempotency_key_hash,
        "deduplicated"
    );
}

/// `executor_missing`: no `ExternalEffectExecutor` is registered for this
/// effect's `effect_type` — fail-closed, loud (spec: "Backward
/// Compatibility").
pub(crate) fn log_executor_missing(effect: &AcceptedEffect) {
    let f = effect_fields(effect);
    warn!(
        effect_id = %f.effect_id,
        effect_type = %f.effect_type,
        destination = %f.destination,
        tenant = %f.tenant,
        idempotency_key_hash = %f.idempotency_key_hash,
        "executor_missing"
    );
}

/// `queue_depth`: how many accepted effects currently sit in the bounded
/// admission queue.
pub(crate) fn log_queue_depth(depth: usize) {
    info!(queue_depth = depth, "queue_depth");
}

/// Computes the millisecond value logged for `oldest_pending_age` — pulled
/// out of `log_oldest_pending_age` so the `None` → `0` mapping is directly
/// testable without emitting through `tracing`.
fn oldest_pending_age_ms(age: Option<Duration>) -> u64 {
    age.map(|a| a.as_millis() as u64).unwrap_or(0)
}

/// `oldest_pending_age`: how long the oldest still-queued effect has been
/// waiting; `None` when nothing is queued.
pub(crate) fn log_oldest_pending_age(age: Option<Duration>) {
    info!(
        oldest_pending_age_ms = oldest_pending_age_ms(age),
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

// --- PROD-002 Phase 2 (AD-14): claim/lease/recovery/cleanup signals ---
//
// A durable provider's own concerns (Postgres claim ownership, provider-owned
// TTL retention) — extending this same `log_*` surface rather than growing a
// parallel one (design.md AD-14). Same discipline as above: no payload, and
// the field-construction stays a pure function separate from the
// `tracing::info!`/`warn!` call, for the same CORE-027 flakiness reason noted
// at the top of this file.

/// Fields for [`log_claim_acquired`], split out so redaction/shape is
/// directly testable without emitting through `tracing`.
struct ClaimAcquiredFields {
    effect_id: String,
    owner: String,
    expires_at: String,
}

fn claim_acquired_fields(
    effect_id: EffectId,
    owner: Uuid,
    expires_at: Timestamp,
) -> ClaimAcquiredFields {
    ClaimAcquiredFields {
        effect_id: effect_id.to_string(),
        owner: owner.to_string(),
        expires_at: expires_at.into_utc().to_rfc3339(),
    }
}

/// `claim_acquired`: a durable provider's `claim_due` stamped ownership
/// (`owner`) and a lease (`expires_at`) on `effect_id` (PROD-002 AD-2/AD-14).
#[allow(dead_code)] // ponytail: wired by the Postgres provider, Phase 5 (PR3)
pub(crate) fn log_claim_acquired(effect_id: EffectId, owner: Uuid, expires_at: Timestamp) {
    let f = claim_acquired_fields(effect_id, owner, expires_at);
    info!(
        effect_id = %f.effect_id,
        owner = %f.owner,
        expires_at = %f.expires_at,
        "claim_acquired"
    );
}

/// Fields for [`log_claim_reclaimed_after_expiry`].
struct ClaimReclaimedFields {
    effect_id: String,
    previous_owner: String,
    new_owner: String,
    previous_epoch: i64,
    new_epoch: i64,
}

fn claim_reclaimed_fields(
    effect_id: EffectId,
    previous_owner: Uuid,
    new_owner: Uuid,
    previous_epoch: i64,
    new_epoch: i64,
) -> ClaimReclaimedFields {
    ClaimReclaimedFields {
        effect_id: effect_id.to_string(),
        previous_owner: previous_owner.to_string(),
        new_owner: new_owner.to_string(),
        previous_epoch,
        new_epoch,
    }
}

/// `claim_reclaimed_after_expiry`: `claim_due` took over a row whose lease
/// had expired — fired from `claim_due` itself, the only place that sees
/// both the prior and new owner/epoch in hand (design.md AD-14/§3.1). Does
/// **not** claim a stale write from the superseded generation can no longer
/// land (§3.1's known G2 limitation) — it only confirms the reclaim itself
/// happened.
#[allow(dead_code)] // ponytail: wired by the Postgres provider, Phase 5 (PR3)
pub(crate) fn log_claim_reclaimed_after_expiry(
    effect_id: EffectId,
    previous_owner: Uuid,
    new_owner: Uuid,
    previous_epoch: i64,
    new_epoch: i64,
) {
    let f = claim_reclaimed_fields(
        effect_id,
        previous_owner,
        new_owner,
        previous_epoch,
        new_epoch,
    );
    warn!(
        effect_id = %f.effect_id,
        previous_owner = %f.previous_owner,
        new_owner = %f.new_owner,
        previous_epoch = f.previous_epoch,
        new_epoch = f.new_epoch,
        "claim_reclaimed_after_expiry"
    );
}

/// Fields for [`log_recovered_in_flight`].
struct RecoveredInFlightFields {
    recovered: u64,
    scope: String,
}

fn recovered_in_flight_fields(recovered: u64, scope: &str) -> RecoveredInFlightFields {
    RecoveredInFlightFields {
        recovered,
        scope: scope.to_string(),
    }
}

/// `recovered_in_flight`: a `recover_in_flight` sweep (startup, or a
/// Postgres provider's expired-lease scope) recovered `recovered` effects
/// (design.md AD-4/AD-14). `scope` names where the sweep ran (e.g.
/// `"startup"`), distinguishing this from `EffectStateStore::
/// recover_in_flight`'s own return count.
#[allow(dead_code)] // ponytail: wired by the delivery runner/providers, Phase 4/5 (PR2/PR3)
pub(crate) fn log_recovered_in_flight(recovered: u64, scope: &str) {
    let f = recovered_in_flight_fields(recovered, scope);
    info!(
        recovered_effect_count = f.recovered,
        scope = %f.scope,
        "recovered_in_flight"
    );
}

/// Fields for [`log_cleanup_deleted`].
struct CleanupDeletedFields {
    deleted: u64,
    table: String,
}

fn cleanup_deleted_fields(deleted: u64, table: &str) -> CleanupDeletedFields {
    CleanupDeletedFields {
        deleted,
        table: table.to_string(),
    }
}

/// `cleanup_deleted`: a provider-owned TTL retention task (design.md AD-9)
/// deleted `deleted` settled rows from `table`. Public — called from the
/// `ego-effect-store` crate's durable providers (Stoolap Phase 4, Postgres
/// Phase 5), which live outside `ego-runtime`.
pub fn log_cleanup_deleted(deleted: u64, table: &str) {
    let f = cleanup_deleted_fields(deleted, table);
    info!(
        deleted_row_count = f.deleted,
        table = %f.table,
        "cleanup_deleted"
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::effects::store::{EffectId, Timestamp};
    use ego_domain::{ExternalEffectDescription, IdempotencyKey, TenantId};
    use std::sync::Arc;
    use uuid::Uuid;

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
    fn effect_fields_carries_required_correlation_values() {
        let effect = sample_effect();
        let f = effect_fields(&effect);

        assert_eq!(f.effect_id, effect.id.to_string());
        assert_eq!(f.effect_type, "invoice.created");
        assert_eq!(f.destination, "https://example.com");
        assert_eq!(f.tenant, "tenant-a");
        assert_eq!(f.idempotency_key_hash, hashed_key("uow-1:0"));
    }

    #[test]
    fn effect_fields_never_leaks_the_payload_or_the_raw_idempotency_key() {
        let effect = sample_effect();
        let f = effect_fields(&effect);

        for value in [
            &f.effect_id,
            &f.effect_type,
            &f.destination,
            &f.tenant,
            &f.idempotency_key_hash,
        ] {
            assert!(
                !value.contains(DISTINCTIVE_PAYLOAD_MARKER),
                "field {value:?} leaked the payload"
            );
            assert_ne!(
                value, "uow-1:0",
                "field {value:?} must never carry the raw idempotency key verbatim"
            );
        }
    }

    #[test]
    fn oldest_pending_age_ms_maps_none_to_zero() {
        assert_eq!(oldest_pending_age_ms(None), 0);
    }

    #[test]
    fn oldest_pending_age_ms_converts_some_duration_to_milliseconds() {
        assert_eq!(oldest_pending_age_ms(Some(Duration::from_secs(1))), 1000);
    }

    // --- PROD-002 Phase 2 (AD-14): claim/lease/recovery/cleanup signals ---

    #[test]
    fn claim_acquired_fields_carries_effect_owner_and_expiry() {
        let effect_id = EffectId::new();
        let owner = Uuid::new_v4();
        let expires_at = Timestamp::now();

        let f = claim_acquired_fields(effect_id, owner, expires_at);

        assert_eq!(f.effect_id, effect_id.to_string());
        assert_eq!(f.owner, owner.to_string());
        assert_eq!(f.expires_at, expires_at.into_utc().to_rfc3339());
    }

    #[test]
    fn claim_reclaimed_fields_carries_both_owners_and_both_epochs() {
        let effect_id = EffectId::new();
        let previous_owner = Uuid::new_v4();
        let new_owner = Uuid::new_v4();

        let f = claim_reclaimed_fields(effect_id, previous_owner, new_owner, 3, 4);

        assert_eq!(f.effect_id, effect_id.to_string());
        assert_eq!(f.previous_owner, previous_owner.to_string());
        assert_eq!(f.new_owner, new_owner.to_string());
        assert_eq!(f.previous_epoch, 3);
        assert_eq!(f.new_epoch, 4);
        assert_ne!(
            f.previous_owner, f.new_owner,
            "a genuine reclaim must carry two distinct owners"
        );
    }

    #[test]
    fn recovered_in_flight_fields_carries_count_and_scope() {
        let f = recovered_in_flight_fields(3, "startup");

        assert_eq!(f.recovered, 3);
        assert_eq!(f.scope, "startup");
    }

    #[test]
    fn recovered_in_flight_fields_reports_zero_when_nothing_recovered() {
        let f = recovered_in_flight_fields(0, "startup");

        assert_eq!(f.recovered, 0);
    }

    #[test]
    fn cleanup_deleted_fields_carries_deleted_count_and_table_name() {
        let f = cleanup_deleted_fields(42, "effect_state");

        assert_eq!(f.deleted, 42);
        assert_eq!(f.table, "effect_state");
    }
}
