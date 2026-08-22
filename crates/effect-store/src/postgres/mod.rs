//! `PostgresEffectStore` — the multi-node-safe durable provider (PROD-002
//! AD-1, AD-2 lease/ownership path).
//!
//! Backed by `sqlx`'s Postgres driver, which is genuinely async — every
//! call in this file is a real `.await`, unlike the Stoolap provider's
//! `spawn_blocking` workaround (Stoolap's driver is synchronous; sqlx's
//! isn't, so no such wrapper is needed here).
//!
//! Ownership lives entirely in the SQL, not the Rust port (design.md §3.1):
//! a `PostgresEffectStore` mints a fresh `worker_id: Uuid` at construction,
//! and `effect_state` carries `claim_owner`/`claim_expires_at`/`claim_epoch`.
//! `claim_due` stamps ownership+lease without transitioning `state` (CORE-019
//! AD-8); every `mark_*` verb is a conditional `UPDATE` guarded by ownership.
//! `claim_epoch` is observability-only — stamped, never checked in a guard
//! (§3.1's accepted G2 limitation: a same-`worker_id` reclaim can still land
//! a stale write; closing it would require threading a token through the
//! port, which AD-6 rejects).
//!
//! `capabilities()` reports `{ durable: true, concurrent_local_safe: true,
//! multi_node_safe: true, supports_leases: true }` (design.md §3.2 table) —
//! the only provider that declares `multi_node_safe`/`supports_leases: true`.
//!
//! ## `mark_in_flight`'s self-claiming guard
//!
//! Every `mark_*` verb after the first is guarded by the literal design.md
//! §3.1 predicate: `claim_owner = $worker_id AND claim_expires_at >
//! $injected_clock_now` — a bound parameter sourced from the store's injected
//! `Clock` (PROD-002 G10), not SQL-side `now()`, so the same instant that
//! computed a claim's `claim_expires_at` also validates it; it assumes
//! `claim_due` already stamped ownership. But the shared Tier 1
//! conformance harness (`tests/conformance.rs`, run identically against
//! every provider) calls `mark_in_flight` directly after `accept`, with no
//! `claim_due` in between — exactly the case a fresh in-memory/Stoolap
//! effect (no ownership concept at all) already supports. So `mark_in_flight`
//! alone uses a broader guard: `claim_owner IS NULL OR claim_owner =
//! $worker_id` (plus the same lease-validity check) — it may *self-claim* an
//! unclaimed row instead of requiring a prior `claim_due` call, while a row
//! validly claimed by a peer still correctly fails. Once a row has state
//! `InFlight`, every subsequent verb (`mark_succeeded`/`mark_retryable`/
//! `mark_terminal`) reverts to the strict guard, matching design.md verbatim.

use std::str::FromStr;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use ego_domain::{Clock, ExternalEffectDescription, IdempotencyKey, TenantId};
use ego_runtime::effects::observability::log_cleanup_deleted;
use ego_runtime::effects::store::{
    AcceptedEffect, DedupOutcome, DedupScope, EffectDedupStore, EffectFingerprint, EffectId,
    EffectState, EffectStateStore, EffectStoreCapabilities, EffectStoreError, StoredEffect,
    TerminalReason, Timestamp,
};
use sqlx::postgres::{PgConnection, PgPoolOptions};
use sqlx::{Connection, PgPool, Row};
use uuid::Uuid;

mod migrations;

fn effect_id_to_uuid(id: EffectId) -> Uuid {
    // `EffectId`'s `Display` always writes its inner UUID verbatim (see
    // `crates/runtime/src/effects/store.rs`) — there is no other public
    // accessor for the inner `Uuid`, so round-tripping through the string
    // form is the only way to get one back out. Same pattern as the Stoolap
    // provider (`stoolap/mod.rs`), just parsed into a real `Uuid` here since
    // Postgres has a native `UUID` column type.
    Uuid::from_str(&id.to_string()).expect("EffectId::to_string always yields a valid UUID")
}

fn state_from_str(s: &str) -> Result<EffectState, EffectStoreError> {
    match s {
        "pending" => Ok(EffectState::Pending),
        "in_flight" => Ok(EffectState::InFlight),
        "succeeded" => Ok(EffectState::Succeeded),
        "retryable_failed" => Ok(EffectState::RetryableFailed),
        "terminal_failed" => Ok(EffectState::TerminalFailed),
        other => Err(EffectStoreError::Backend(format!(
            "unknown persisted state {other:?}"
        ))),
    }
}

fn encode_reason(reason: &TerminalReason) -> String {
    match reason {
        TerminalReason::ExecutorMissing => "executor_missing".to_string(),
        TerminalReason::InvalidEffect(msg) => format!("invalid_effect:{msg}"),
        TerminalReason::Other(msg) => format!("other:{msg}"),
    }
}

/// Classifies a raw `sqlx::Error`. Only genuinely transient conditions
/// (connection pool exhaustion, serialization failures, deadlocks, lock
/// timeouts, transport I/O) map to [`EffectStoreError::TemporarilyUnavailable`]
/// — the only variant `acceptor.rs`/`runner.rs` retry. Every other error
/// (constraint violations the caller didn't already handle, corrupt data,
/// schema mismatch, ...) is a permanent [`EffectStoreError::Backend`]
/// failure, never automatically retried. Mirrors the Stoolap provider's
/// `backend_err` classification discipline (correction-round fix 2).
fn backend_err(e: sqlx::Error) -> EffectStoreError {
    match &e {
        // Connection pool exhaustion / transport failure: always transient.
        sqlx::Error::PoolTimedOut | sqlx::Error::Io(_) | sqlx::Error::PoolClosed => {
            EffectStoreError::TemporarilyUnavailable(e.to_string())
        }
        sqlx::Error::Database(db_err) => match db_err.code().as_deref() {
            // serialization_failure, deadlock_detected, too_many_connections,
            // lock_not_available — all genuinely transient PostgreSQL
            // conditions, safe to retry.
            Some("40001") | Some("40P01") | Some("53300") | Some("55P03") => {
                EffectStoreError::TemporarilyUnavailable(e.to_string())
            }
            _ => EffectStoreError::Backend(e.to_string()),
        },
        _ => EffectStoreError::Backend(e.to_string()),
    }
}

/// The dedup row this call just inserted-or-conflicted on is missing when
/// re-selected a moment later: a peer's `release()` landed in the gap
/// between the `INSERT ... ON CONFLICT DO NOTHING` and this follow-up
/// `SELECT`. Genuinely transient — the caller should retry `reserve`, which
/// will then correctly observe `Fresh` — never a permanent backend failure
/// (mirrors the Stoolap provider's identical fix).
fn dedup_row_vanished_error() -> EffectStoreError {
    EffectStoreError::TemporarilyUnavailable(
        "dedup row vanished after ON CONFLICT DO NOTHING — retry reserve".into(),
    )
}

fn validate_schema_identifier(schema: &str) -> Result<(), EffectStoreError> {
    if schema.is_empty()
        || !schema
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_')
    {
        return Err(EffectStoreError::Backend(format!(
            "invalid schema identifier: {schema:?} (must be non-empty ASCII alphanumeric/underscore)"
        )));
    }
    Ok(())
}

/// A [`PostgresEffectStore::run_retention`] batch that failed partway
/// through. Carries how many rows this call had already deleted (across
/// both tables) before `source` occurred, so a caller can tell "deleted N
/// of the batch, then errored" from "deleted nothing" — mirrors the Stoolap
/// provider's `PartialRetentionFailure` (correction-round fix 5).
#[derive(Debug)]
pub struct PostgresRetentionFailure {
    /// Rows deleted in this call before `source` occurred.
    pub deleted: u64,
    /// The underlying delete failure.
    pub source: EffectStoreError,
}

impl std::fmt::Display for PostgresRetentionFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "retention batch failed after deleting {} rows: {}",
            self.deleted, self.source
        )
    }
}

impl std::error::Error for PostgresRetentionFailure {}

/// A durable, multi-node-safe [`EffectStateStore`]/[`EffectDedupStore`]
/// implementation backed by PostgreSQL (design.md AD-2/AD-5/AD-6).
pub struct PostgresEffectStore {
    pool: PgPool,
    worker_id: Uuid,
    lease: Duration,
    clock: Arc<dyn Clock>,
}

impl PostgresEffectStore {
    /// Connects to `database_url`, ensuring `schema` exists and scoping
    /// every pooled connection's `search_path` to it (so callers can run
    /// several independent `PostgresEffectStore`s against one physical
    /// database, each with its own tables — the shape the Tier 2/3
    /// conformance factory needs), then runs this crate's own migration
    /// sequence (AD-10) and mints a fresh `worker_id`.
    ///
    /// `lease` is this store's claim lease duration (design.md §6: must
    /// comfortably exceed one dispatch's worst-case duration). `clock` is
    /// the injectable time source (PROD-002 G10) used to compute
    /// `claim_expires_at` and to validate it in every `mark_*` guard — a
    /// production caller should pass `Arc::new(SystemClock)`; a test can
    /// inject a deterministic double instead of relying on wall-clock sleeps.
    pub async fn connect(
        database_url: &str,
        schema: &str,
        lease: Duration,
        clock: Arc<dyn Clock>,
    ) -> Result<Self, EffectStoreError> {
        validate_schema_identifier(schema)?;

        // Ensure the schema exists before any pooled connection tries to
        // `SET search_path` to it below.
        {
            let mut conn = PgConnection::connect(database_url)
                .await
                .map_err(backend_err)?;
            // security review: identifiers can't be `$N`-bound in Postgres;
            // `schema` was allowlisted (ASCII alphanumeric/underscore only)
            // by `validate_schema_identifier` above, so this interpolation
            // is injection-safe.
            sqlx::query(&format!("CREATE SCHEMA IF NOT EXISTS \"{schema}\""))
                .execute(&mut conn)
                .await
                .map_err(backend_err)?;
        }

        let schema_owned = schema.to_string();
        let pool = PgPoolOptions::new()
            .after_connect(move |conn, _meta| {
                let schema = schema_owned.clone();
                Box::pin(async move {
                    // security review: same allowlisted `schema` as above,
                    // already validated by `validate_schema_identifier`
                    // before `connect` ever reaches this closure —
                    // injection-safe for the same reason.
                    sqlx::query(&format!("SET search_path TO \"{schema}\", public"))
                        .execute(&mut *conn)
                        .await?;
                    Ok(())
                })
            })
            .connect(database_url)
            .await
            .map_err(backend_err)?;

        migrations::run(&pool).await.map_err(backend_err)?;

        Ok(Self {
            pool,
            worker_id: Uuid::new_v4(),
            lease,
            clock,
        })
    }

    /// Classifies why a conditional transition affected zero rows.
    ///
    /// Deliberately returns `Result<EffectStoreError, EffectStoreError>`:
    /// the `Ok` value is the classified transition error to bubble up to
    /// the caller (`NotFound`/`InvalidTransition`/`Conflict`), while `Err`
    /// is a genuine backend query failure encountered while classifying.
    /// `allowed_from` is the transition's own legal source states (AD-5):
    /// if the row's actual current state isn't in it, that alone explains
    /// the zero-row `UPDATE` (`InvalidTransition`); otherwise the state
    /// matched, so it must have been the ownership/lease guard (`Conflict`
    /// — design.md §3.1: "a worker whose row was reclaimed by a peer sees
    /// rows_affected == 0").
    async fn transition_error(
        &self,
        id: EffectId,
        allowed_from: &[EffectState],
        to: EffectState,
    ) -> Result<EffectStoreError, EffectStoreError> {
        let row = sqlx::query("SELECT state FROM effect_state WHERE effect_id = $1")
            .bind(effect_id_to_uuid(id))
            .fetch_optional(&self.pool)
            .await
            .map_err(backend_err)?;

        let Some(row) = row else {
            return Ok(EffectStoreError::NotFound(id));
        };

        let state_str: String = row.try_get("state").map_err(backend_err)?;
        let from = state_from_str(&state_str)?;

        if !allowed_from.contains(&from) {
            return Ok(EffectStoreError::InvalidTransition { id, from, to });
        }

        Ok(EffectStoreError::Conflict(format!(
            "effect {id} is claimed by another worker, or its lease has expired"
        )))
    }

    /// Provider-owned TTL retention (AD-9): deletes settled `effect_state`
    /// rows (`Succeeded`/`TerminalFailed`) and settled `effect_dedup` rows
    /// older than `ttl`, in bounded batches. Not a port operation — the
    /// delivery ports expose no purge verb (design §3, AD-9).
    ///
    /// Each table's delete is one atomic statement (a read-only CTE feeding
    /// the `DELETE`'s own `WHERE`), so there is no separate SELECT-then-
    /// DELETE round trip during which a scope could change hands — but the
    /// `effect_dedup` delete's `WHERE` clause still **re-checks**
    /// `succeeded`/`settled_at < cutoff` against the row's current values
    /// (not just its scope-key identity), matching the Stoolap provider's
    /// TOCTOU fix (correction-round fix 1): a `release()` + fresh
    /// `reserve()` landing on the exact same `(tenant, effect_type,
    /// idempotency_key)` between this call's snapshot and delete must never
    /// delete the new, live, non-terminal reservation.
    pub async fn run_retention(
        &self,
        now: Timestamp,
        ttl: Duration,
        batch: i64,
    ) -> Result<u64, PostgresRetentionFailure> {
        let cutoff = now.into_utc() - ttl;
        let mut deleted = 0u64;

        let state_result = sqlx::query(
            "WITH eligible AS (
                SELECT effect_id FROM effect_state
                WHERE state IN ('succeeded', 'terminal_failed')
                  AND settled_at IS NOT NULL AND settled_at < $1
                LIMIT $2
            )
            DELETE FROM effect_state
            WHERE effect_id IN (SELECT effect_id FROM eligible)
              AND state IN ('succeeded', 'terminal_failed')
              AND settled_at IS NOT NULL AND settled_at < $1",
        )
        .bind(cutoff)
        .bind(batch)
        .execute(&self.pool)
        .await
        .map_err(|e| PostgresRetentionFailure {
            deleted,
            source: backend_err(e),
        })?;
        deleted += state_result.rows_affected();

        let dedup_result = sqlx::query(
            "WITH eligible AS (
                SELECT tenant_id, effect_type, idempotency_key FROM effect_dedup
                WHERE succeeded = true
                  AND settled_at IS NOT NULL AND settled_at < $1
                LIMIT $2
            )
            DELETE FROM effect_dedup d
            USING eligible e
            WHERE d.tenant_id = e.tenant_id
              AND d.effect_type = e.effect_type
              AND d.idempotency_key = e.idempotency_key
              AND d.succeeded = true
              AND d.settled_at IS NOT NULL AND d.settled_at < $1",
        )
        .bind(cutoff)
        .bind(batch)
        .execute(&self.pool)
        .await
        .map_err(|e| PostgresRetentionFailure {
            deleted,
            source: backend_err(e),
        })?;
        deleted += dedup_result.rows_affected();

        if deleted > 0 {
            log_cleanup_deleted(deleted, "effect_state+effect_dedup");
        }
        Ok(deleted)
    }
}

#[async_trait]
impl EffectStateStore for PostgresEffectStore {
    fn capabilities(&self) -> EffectStoreCapabilities {
        EffectStoreCapabilities {
            durable: true,
            concurrent_local_safe: true,
            multi_node_safe: true,
            supports_leases: true,
        }
    }

    async fn accept(&self, effect: AcceptedEffect) -> Result<(), EffectStoreError> {
        let id_uuid = effect_id_to_uuid(effect.id);
        let affected = sqlx::query(
            "INSERT INTO effect_state
                (effect_id, tenant_id, effect_type, destination, idempotency_key,
                 payload, attempt, state)
             VALUES ($1, $2, $3, $4, $5, $6, $7, 'pending')
             ON CONFLICT (effect_id) DO NOTHING",
        )
        .bind(id_uuid)
        .bind(effect.tenant.as_str())
        .bind(&effect.description.effect_type)
        .bind(&effect.description.destination)
        .bind(effect.description.idempotency_key.as_str())
        .bind(&effect.description.payload)
        .bind(effect.attempt as i32)
        .execute(&self.pool)
        .await
        .map_err(backend_err)?
        .rows_affected();

        if affected == 1 {
            return Ok(());
        }

        // Already present — classify replay-vs-conflict (idempotent accept).
        let row = sqlx::query(
            "SELECT tenant_id, effect_type, destination, idempotency_key, payload
             FROM effect_state WHERE effect_id = $1",
        )
        .bind(id_uuid)
        .fetch_optional(&self.pool)
        .await
        .map_err(backend_err)?
        .ok_or(EffectStoreError::NotFound(effect.id))?;

        let existing_tenant: String = row.try_get("tenant_id").map_err(backend_err)?;
        let existing_type: String = row.try_get("effect_type").map_err(backend_err)?;
        let existing_dest: String = row.try_get("destination").map_err(backend_err)?;
        let existing_key: String = row.try_get("idempotency_key").map_err(backend_err)?;
        let existing_payload: Vec<u8> = row.try_get("payload").map_err(backend_err)?;

        if existing_tenant == effect.tenant.as_str()
            && existing_type == effect.description.effect_type
            && existing_dest == effect.description.destination
            && existing_key == effect.description.idempotency_key.as_str()
            && existing_payload == effect.description.payload
        {
            Ok(())
        } else {
            Err(EffectStoreError::Conflict(format!(
                "effect {} already accepted with different tenant/description",
                effect.id
            )))
        }
    }

    async fn mark_in_flight(&self, id: EffectId) -> Result<(), EffectStoreError> {
        let id_uuid = effect_id_to_uuid(id);
        let now = self.clock.now();
        let expires_at = now + self.lease;

        let affected = sqlx::query(
            "UPDATE effect_state
             SET state = 'in_flight', claim_owner = $2, claim_expires_at = $3
             WHERE effect_id = $1
               AND state IN ('pending', 'retryable_failed')
               AND (claim_owner IS NULL OR claim_owner = $2)
               AND (claim_expires_at IS NULL OR claim_expires_at > $4)",
        )
        .bind(id_uuid)
        .bind(self.worker_id)
        .bind(expires_at)
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(backend_err)?
        .rows_affected();

        if affected == 1 {
            Ok(())
        } else {
            Err(self
                .transition_error(
                    id,
                    &[EffectState::Pending, EffectState::RetryableFailed],
                    EffectState::InFlight,
                )
                .await?)
        }
    }

    async fn mark_succeeded(&self, id: EffectId) -> Result<(), EffectStoreError> {
        let id_uuid = effect_id_to_uuid(id);

        let affected = sqlx::query(
            "UPDATE effect_state
             SET state = 'succeeded', settled_at = now()
             WHERE effect_id = $1
               AND state = 'in_flight'
               AND claim_owner = $2
               AND claim_expires_at > $3",
        )
        .bind(id_uuid)
        .bind(self.worker_id)
        .bind(self.clock.now())
        .execute(&self.pool)
        .await
        .map_err(backend_err)?
        .rows_affected();

        if affected == 1 {
            Ok(())
        } else {
            Err(self
                .transition_error(id, &[EffectState::InFlight], EffectState::Succeeded)
                .await?)
        }
    }

    async fn mark_retryable(
        &self,
        id: EffectId,
        attempt: u32,
        next_at: Timestamp,
    ) -> Result<(), EffectStoreError> {
        let id_uuid = effect_id_to_uuid(id);

        // Clears claim_owner/claim_expires_at on success: the active claim's
        // job (guarding a dispatching attempt) is done once the effect is
        // back to `RetryableFailed`, waiting on `next_at` — the row must be
        // freely reclaimable by claim_due's normal G1-guarded pending/
        // retryable_failed branch on its next due tick, not held hostage by
        // a stale lease from the attempt that just failed (that lease could
        // otherwise still have most of its duration left, blocking every
        // claim_due call — including this same worker's own next tick —
        // from ever re-claiming a row that is no longer actively in flight).
        let affected = sqlx::query(
            "UPDATE effect_state
             SET state = 'retryable_failed', attempt = $2, next_at = $3,
                 claim_owner = NULL, claim_expires_at = NULL
             WHERE effect_id = $1
               AND state = 'in_flight'
               AND claim_owner = $4
               AND claim_expires_at > $5",
        )
        .bind(id_uuid)
        .bind(attempt as i32)
        .bind(next_at.into_utc())
        .bind(self.worker_id)
        .bind(self.clock.now())
        .execute(&self.pool)
        .await
        .map_err(backend_err)?
        .rows_affected();

        if affected == 1 {
            Ok(())
        } else {
            Err(self
                .transition_error(id, &[EffectState::InFlight], EffectState::RetryableFailed)
                .await?)
        }
    }

    async fn mark_terminal(
        &self,
        id: EffectId,
        reason: TerminalReason,
    ) -> Result<(), EffectStoreError> {
        let id_uuid = effect_id_to_uuid(id);
        let encoded = encode_reason(&reason);

        let affected = sqlx::query(
            "UPDATE effect_state
             SET state = 'terminal_failed', terminal_reason = $2, settled_at = now()
             WHERE effect_id = $1
               AND state IN ('in_flight', 'retryable_failed')
               AND claim_owner = $3
               AND claim_expires_at > $4",
        )
        .bind(id_uuid)
        .bind(encoded)
        .bind(self.worker_id)
        .bind(self.clock.now())
        .execute(&self.pool)
        .await
        .map_err(backend_err)?
        .rows_affected();

        if affected == 1 {
            Ok(())
        } else {
            Err(self
                .transition_error(
                    id,
                    &[EffectState::InFlight, EffectState::RetryableFailed],
                    EffectState::TerminalFailed,
                )
                .await?)
        }
    }

    async fn claim_due(
        &self,
        now: Timestamp,
        limit: usize,
    ) -> Result<Vec<StoredEffect>, EffectStoreError> {
        let now_utc = now.into_utc();
        let expires_at = now_utc + self.lease;

        // design.md §3.1's exact claim_due SQL, with the G1 fix (the
        // pending/retryable branch's `claim_owner IS NULL OR
        // claim_expires_at < $now` guard — without it a second claim_due
        // call could re-stamp a row a live claim already covers, since
        // claim_due deliberately does not transition `state`) plus a
        // NULL-safe `next_at` check (a freshly accepted row's `next_at` is
        // NULL, matching the Stoolap/in-memory providers' `claim_due`).
        let rows = sqlx::query(
            "UPDATE effect_state
             SET claim_owner = $1,
                 claim_epoch = claim_epoch + 1,
                 claim_expires_at = $2
             WHERE effect_id IN (
                 SELECT effect_id FROM effect_state
                 WHERE (
                     state IN ('pending', 'retryable_failed')
                     AND (next_at IS NULL OR next_at <= $3)
                     AND (claim_owner IS NULL OR claim_expires_at < $3)
                 ) OR (
                     state = 'in_flight'
                     AND claim_expires_at < $3
                 )
                 ORDER BY next_at
                 FOR UPDATE SKIP LOCKED
                 LIMIT $4
             )
             RETURNING effect_id, tenant_id, effect_type, destination, idempotency_key,
                       payload, attempt, state, next_at",
        )
        .bind(self.worker_id)
        .bind(expires_at)
        .bind(now_utc)
        .bind(limit as i64)
        .fetch_all(&self.pool)
        .await
        .map_err(backend_err)?;

        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            let id_uuid: Uuid = row.try_get("effect_id").map_err(backend_err)?;
            let tenant: String = row.try_get("tenant_id").map_err(backend_err)?;
            let effect_type: String = row.try_get("effect_type").map_err(backend_err)?;
            let destination: String = row.try_get("destination").map_err(backend_err)?;
            let idempotency_key: String = row.try_get("idempotency_key").map_err(backend_err)?;
            let payload: Vec<u8> = row.try_get("payload").map_err(backend_err)?;
            let attempt: i32 = row.try_get("attempt").map_err(backend_err)?;
            let state_str: String = row.try_get("state").map_err(backend_err)?;
            let next_at: Option<DateTime<Utc>> = row.try_get("next_at").map_err(backend_err)?;

            out.push(StoredEffect {
                id: EffectId::from_uuid(id_uuid),
                tenant: TenantId::new(tenant)
                    .map_err(|e| EffectStoreError::Backend(format!("tenant decode: {e}")))?,
                description: std::sync::Arc::new(ExternalEffectDescription {
                    idempotency_key: IdempotencyKey::new(idempotency_key).map_err(|e| {
                        EffectStoreError::Backend(format!("idempotency_key decode: {e}"))
                    })?,
                    effect_type,
                    payload,
                    destination,
                }),
                attempt: attempt as u32,
                state: state_from_str(&state_str)?,
                next_at: next_at.map(Timestamp::from_utc).unwrap_or(now),
            });
        }
        Ok(out)
    }

    async fn recover_in_flight(&self, now: Timestamp) -> Result<u64, EffectStoreError> {
        // Scoped to expired-lease rows only (design.md §3.3/AD-4): a
        // restarting node must never steal an effect a live peer currently
        // owns — unlike the single-owner in-memory/Stoolap providers'
        // blanket reset.
        let affected = sqlx::query(
            "UPDATE effect_state
             SET state = 'pending', next_at = $1
             WHERE state = 'in_flight' AND claim_expires_at < $1",
        )
        .bind(now.into_utc())
        .execute(&self.pool)
        .await
        .map_err(backend_err)?
        .rows_affected();

        Ok(affected)
    }
}

#[async_trait]
impl EffectDedupStore for PostgresEffectStore {
    fn capabilities(&self) -> EffectStoreCapabilities {
        EffectStoreCapabilities {
            durable: true,
            concurrent_local_safe: true,
            multi_node_safe: true,
            supports_leases: true,
        }
    }

    async fn reserve(
        &self,
        scope: &DedupScope,
        effect_id: EffectId,
        fingerprint: EffectFingerprint,
    ) -> Result<DedupOutcome, EffectStoreError> {
        let id_uuid = effect_id_to_uuid(effect_id);
        let affected = sqlx::query(
            "INSERT INTO effect_dedup
                (tenant_id, effect_type, idempotency_key, effect_id, fingerprint, succeeded)
             VALUES ($1, $2, $3, $4, $5, false)
             ON CONFLICT (tenant_id, effect_type, idempotency_key) DO NOTHING",
        )
        .bind(scope.tenant.as_str())
        .bind(&scope.effect_type)
        .bind(scope.key.as_str())
        .bind(id_uuid)
        .bind(fingerprint.as_bytes().as_slice())
        .execute(&self.pool)
        .await
        .map_err(backend_err)?
        .rows_affected();

        if affected == 1 {
            return Ok(DedupOutcome::Fresh);
        }

        let row = sqlx::query(
            "SELECT effect_id, fingerprint, succeeded FROM effect_dedup
             WHERE tenant_id = $1 AND effect_type = $2 AND idempotency_key = $3",
        )
        .bind(scope.tenant.as_str())
        .bind(&scope.effect_type)
        .bind(scope.key.as_str())
        .fetch_optional(&self.pool)
        .await
        .map_err(backend_err)?
        .ok_or_else(dedup_row_vanished_error)?;

        let existing_owner: Uuid = row.try_get("effect_id").map_err(backend_err)?;
        let existing_fp: Vec<u8> = row.try_get("fingerprint").map_err(backend_err)?;
        let succeeded: bool = row.try_get("succeeded").map_err(backend_err)?;

        if existing_fp != fingerprint.as_bytes().as_slice() {
            return Ok(DedupOutcome::Conflict);
        }
        if existing_owner == id_uuid {
            if succeeded {
                Ok(DedupOutcome::OwnedSucceeded)
            } else {
                Ok(DedupOutcome::OwnedInProgress)
            }
        } else if succeeded {
            Ok(DedupOutcome::OtherSucceeded)
        } else {
            Ok(DedupOutcome::OtherInProgress)
        }
    }

    async fn commit_success(&self, scope: &DedupScope) -> Result<(), EffectStoreError> {
        sqlx::query(
            "UPDATE effect_dedup SET succeeded = true, settled_at = now()
             WHERE tenant_id = $1 AND effect_type = $2 AND idempotency_key = $3",
        )
        .bind(scope.tenant.as_str())
        .bind(&scope.effect_type)
        .bind(scope.key.as_str())
        .execute(&self.pool)
        .await
        .map_err(backend_err)?;
        Ok(())
    }

    async fn release(&self, scope: &DedupScope) -> Result<(), EffectStoreError> {
        sqlx::query(
            "DELETE FROM effect_dedup
             WHERE tenant_id = $1 AND effect_type = $2 AND idempotency_key = $3",
        )
        .bind(scope.tenant.as_str())
        .bind(&scope.effect_type)
        .bind(scope.key.as_str())
        .execute(&self.pool)
        .await
        .map_err(backend_err)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- backend_err classification (deterministic, no DB needed) ---

    #[test]
    fn backend_err_classifies_pool_timeout_as_temporarily_unavailable() {
        let err = backend_err(sqlx::Error::PoolTimedOut);
        assert!(
            matches!(err, EffectStoreError::TemporarilyUnavailable(_)),
            "connection pool exhaustion must be retryable, got {err:?}"
        );
    }

    #[test]
    fn backend_err_classifies_pool_closed_as_temporarily_unavailable() {
        let err = backend_err(sqlx::Error::PoolClosed);
        assert!(
            matches!(err, EffectStoreError::TemporarilyUnavailable(_)),
            "a closed pool must be retryable, got {err:?}"
        );
    }

    #[test]
    fn backend_err_classifies_row_not_found_as_backend_not_transient() {
        let err = backend_err(sqlx::Error::RowNotFound);
        assert!(
            matches!(err, EffectStoreError::Backend(_)),
            "a non-transient sqlx error must stay Backend, got {err:?}"
        );
    }

    #[test]
    fn dedup_row_vanished_error_is_temporarily_unavailable() {
        let err = dedup_row_vanished_error();
        assert!(
            matches!(err, EffectStoreError::TemporarilyUnavailable(_)),
            "a peer's release() racing the INSERT must be retryable, got {err:?}"
        );
    }

    // --- state_from_str: an unrecognized persisted state must not panic ---

    #[test]
    fn state_from_str_rejects_unknown_value_without_panicking() {
        let err = state_from_str("bogus_state").unwrap_err();
        assert!(
            matches!(err, EffectStoreError::Backend(_)),
            "an unrecognized persisted state must be a classified Backend error, not a panic, got {err:?}"
        );
    }

    // --- schema identifier validation ---

    #[test]
    fn validate_schema_identifier_accepts_alphanumeric_and_underscore() {
        assert!(validate_schema_identifier("effect_test_abc123").is_ok());
    }

    #[test]
    fn validate_schema_identifier_rejects_empty_and_injection_attempts() {
        assert!(validate_schema_identifier("").is_err());
        assert!(validate_schema_identifier("public\"; DROP TABLE effect_state; --").is_err());
        assert!(validate_schema_identifier("has space").is_err());
    }

    // --- effect_id_to_uuid round-trip ---

    #[test]
    fn effect_id_to_uuid_round_trips_through_from_uuid() {
        let uuid = Uuid::new_v4();
        let id = EffectId::from_uuid(uuid);
        assert_eq!(effect_id_to_uuid(id), uuid);
    }

    // Tests that need a live Postgres (claim/lease/reclaim/retention/
    // capabilities behavior) live in `crates/integration-tests/tests/
    // effect_store_postgres_unit.rs`, provisioned via testcontainers
    // (ego-rs-testing: a real external resource never runs inline in a
    // production crate's own test module).
}
