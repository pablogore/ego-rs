//! `StoolapEffectStore` — embedded, single-host durable provider (PROD-002
//! AD-1, AD-2 local-ownership path).
//!
//! Backed by [Stoolap](https://stoolap.io), an embedded, pure-Rust MVCC SQL
//! engine. Single-host, single-owner (design.md §3.1): every transition is a
//! plain conditional `UPDATE ... WHERE state IN (...)` under Stoolap's
//! snapshot isolation — **no** owner/epoch/lease columns exist in its schema,
//! unlike the PostgreSQL provider's multi-node path. `capabilities()` reports
//! `{ durable: true, concurrent_local_safe: true, multi_node_safe: false,
//! supports_leases: false }` (design.md §3.2 table).
//!
//! Dialect note: Stoolap's `core::Value` has no binary/BYTEA variant, so
//! `payload` bytes are stored as base64-encoded `TEXT` rather than the raw
//! bytes a Postgres `BYTEA` column would hold.
//!
//! ## Blocking I/O (correction round, resilience fix 4)
//!
//! Every `Database::execute`/`query` call in this file is synchronous,
//! disk-touching I/O — there is no `.await` point anywhere in this module.
//! Left unwrapped, that would stall unrelated async tasks sharing the same
//! Tokio worker thread, directly contradicting `capabilities().
//! concurrent_local_safe: true`. All trait methods therefore run their body
//! via [`StoolapEffectStore::run_blocking`], which hands the closure to
//! [`tokio::task::spawn_blocking`].
//!
//! **Rejected alternative**: `tokio::task::block_in_place`. It requires a
//! multi-threaded Tokio runtime and panics outside one — this crate's own
//! conformance suite (`tests/conformance.rs`) runs its Stoolap tests under
//! plain `#[tokio::test]` (Tokio's default **current-thread** flavor), so
//! `block_in_place` would crash every existing test. `spawn_blocking` works
//! under any runtime flavor and needs only a cheaply, independently clonable
//! handle across the thread boundary — `Database` already provides that (it
//! wraps an internal `Arc`; see its own `impl Clone`), so no additional
//! `Arc<Database>` wrapper is needed in this struct.

use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;
use base64::Engine as _;
use chrono::{DateTime, Duration, Utc};
use ego_domain::{ExternalEffectDescription, IdempotencyKey, TenantId};
use ego_runtime::effects::observability::log_cleanup_deleted;
use ego_runtime::effects::store::{
    AcceptedEffect, DedupOutcome, DedupScope, EffectDedupStore, EffectFingerprint, EffectId,
    EffectState, EffectStateStore, EffectStoreCapabilities, EffectStoreError, RetentionMaintenance,
    StoredEffect, TerminalReason, Timestamp,
};
use stoolap::Database;

const BASE64: base64::engine::general_purpose::GeneralPurpose = base64::engine::general_purpose::STANDARD;

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

fn hex_encode(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        write!(out, "{b:02x}").unwrap();
    }
    out
}

/// Classifies a raw Stoolap error (fix 2, resilience review).
///
/// Only genuinely transient variants (`LockAcquisitionFailed`,
/// `DatabaseLocked`) map to [`EffectStoreError::TemporarilyUnavailable`] —
/// the only variant `acceptor.rs`/`runner.rs` retry. Every other variant
/// (corruption, schema mismatch, missing table, ...) is a permanent
/// [`EffectStoreError::Backend`] failure, never automatically retried.
fn backend_err(e: stoolap::Error) -> EffectStoreError {
    match &e {
        stoolap::Error::LockAcquisitionFailed(_) | stoolap::Error::DatabaseLocked => {
            EffectStoreError::TemporarilyUnavailable(e.to_string())
        }
        _ => EffectStoreError::Backend(e.to_string()),
    }
}

/// The dedup row this call just inserted-or-conflicted on is missing when
/// re-selected a moment later (fix 2, resilience review): a peer's
/// `release()` landed in the gap between the `INSERT ... ON CONFLICT DO
/// NOTHING` and this follow-up `SELECT`. Genuinely transient — the caller
/// should retry `reserve`, which will then correctly observe `Fresh` —
/// never a permanent backend failure.
fn dedup_row_vanished_error() -> EffectStoreError {
    EffectStoreError::TemporarilyUnavailable(
        "dedup row vanished after ON CONFLICT DO NOTHING — retry reserve".into(),
    )
}

/// A [`StoolapEffectStore::run_retention`] batch that failed partway through
/// (fix 5, resilience review). Carries how many rows this call had already
/// deleted before `source` occurred, so a caller can tell "deleted N of the
/// batch, then errored" from "deleted nothing" — a bare `?` per row would
/// otherwise discard that count entirely once the error propagated out.
#[derive(Debug)]
pub struct PartialRetentionFailure {
    /// Rows deleted in this call before `source` occurred.
    pub deleted: u64,
    /// The underlying per-row delete (or select) failure.
    pub source: EffectStoreError,
}

impl std::fmt::Display for PartialRetentionFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "retention batch failed after deleting {} rows: {}", self.deleted, self.source)
    }
}

impl std::error::Error for PartialRetentionFailure {}

/// Applies `delete_one` to each item in `items`, accumulating the running
/// count into `deleted_so_far`. On the first failure, returns a
/// [`PartialRetentionFailure`] carrying whatever had already been
/// accumulated — the count is never silently dropped just because a later
/// row's delete failed (fix 5, resilience review).
fn delete_batch_preserving_partial_count<T>(
    items: Vec<T>,
    deleted_so_far: &mut u64,
    mut delete_one: impl FnMut(&T) -> Result<i64, EffectStoreError>,
) -> Result<(), PartialRetentionFailure> {
    for item in &items {
        match delete_one(item) {
            Ok(affected) => *deleted_so_far += affected.max(0) as u64,
            Err(source) => {
                return Err(PartialRetentionFailure {
                    deleted: *deleted_so_far,
                    source,
                });
            }
        }
    }
    Ok(())
}

/// A durable, single-host [`EffectStateStore`]/[`EffectDedupStore`]
/// implementation backed by an embedded Stoolap database.
pub struct StoolapEffectStore {
    db: Database,
}

impl StoolapEffectStore {
    /// Opens (creating if absent) a Stoolap-backed store at `path`.
    ///
    /// Calling this again at the same `path` after the previous
    /// [`StoolapEffectStore`] has been dropped genuinely reopens the on-disk
    /// database (design §3.6 Tier 2) — Stoolap's process-global registry
    /// only shares a live engine while a handle for that DSN is still alive.
    pub async fn open(path: &Path) -> Result<Self, EffectStoreError> {
        let dsn = format!("file://{}", path.display());
        let db = Database::open(&dsn).map_err(backend_err)?;

        // Dialect note (design.md §6, Stoolap fidelity gate): Stoolap only
        // supports a single-column INTEGER PRIMARY KEY — a TEXT PK (as this
        // schema would naturally use for a UUID id) is rejected at DDL time,
        // and a table-level composite `PRIMARY KEY (...)` is parsed but
        // silently NOT enforced (no constraint, no index — confirmed by
        // reading `executor/ddl.rs`). `UNIQUE (...)` IS fully enforced
        // (single- and multi-column) and is what `ON CONFLICT` matches
        // against, so uniqueness is expressed via `UNIQUE` here instead of
        // `PRIMARY KEY`.
        db.execute(
            "CREATE TABLE IF NOT EXISTS effect_state (
                effect_id TEXT NOT NULL,
                tenant TEXT NOT NULL,
                effect_type TEXT NOT NULL,
                destination TEXT NOT NULL,
                idempotency_key TEXT NOT NULL,
                payload TEXT NOT NULL,
                attempt INTEGER NOT NULL,
                state TEXT NOT NULL,
                next_at TIMESTAMP,
                terminal_reason TEXT,
                settled_at TIMESTAMP,
                UNIQUE (effect_id)
            )",
            (),
        )
        .map_err(backend_err)?;

        db.execute(
            "CREATE TABLE IF NOT EXISTS effect_dedup (
                tenant TEXT NOT NULL,
                effect_type TEXT NOT NULL,
                idempotency_key TEXT NOT NULL,
                effect_id TEXT NOT NULL,
                fingerprint TEXT NOT NULL,
                succeeded BOOLEAN NOT NULL,
                settled_at TIMESTAMP,
                UNIQUE (tenant, effect_type, idempotency_key)
            )",
            (),
        )
        .map_err(backend_err)?;

        Ok(Self { db })
    }

    /// Runs `f` against a cloned [`Database`] handle on Tokio's
    /// blocking-thread pool (see the module-level doc comment for the
    /// `spawn_blocking` vs. `block_in_place` rationale).
    async fn run_blocking<F, R>(&self, f: F) -> Result<R, EffectStoreError>
    where
        F: FnOnce(&Database) -> Result<R, EffectStoreError> + Send + 'static,
        R: Send + 'static,
    {
        let db = self.db.clone();
        tokio::task::spawn_blocking(move || f(&db))
            .await
            .map_err(|e| EffectStoreError::Backend(format!("blocking task panicked: {e}")))?
    }

    /// Provider-owned TTL retention (AD-9): deletes settled `effect_state`
    /// rows (`Succeeded`/`TerminalFailed`) and settled `effect_dedup` rows
    /// older than `ttl`, in bounded batches. Not a port operation — the
    /// delivery ports expose no purge verb (design §3, AD-9).
    ///
    /// Dialect note (design.md §6, Stoolap fidelity gate): a single
    /// `DELETE ... WHERE col IN (SELECT ... LIMIT n)` — the natural
    /// batched-delete shape design.md assumed — silently deletes **zero**
    /// rows against Stoolap 0.4.0, even with a literal (unparameterized)
    /// subquery; confirmed by direct experiment, not a parameter-binding
    /// issue. `DELETE ... WHERE col IN (<value list>)` and `DELETE ... WHERE
    /// col = $1` both work correctly, so batching here is done in two steps:
    /// select the bounded batch of eligible identifiers, then delete each by
    /// its own equality predicate.
    pub async fn run_retention(
        &self,
        now: Timestamp,
        ttl: Duration,
        batch: i64,
    ) -> Result<u64, PartialRetentionFailure> {
        let cutoff = now.into_utc() - ttl;
        let db = self.db.clone();

        tokio::task::spawn_blocking(move || {
            let db = &db;
            let mut deleted = 0u64;

            let effect_ids: Vec<String> = {
                let rows = db
                    .query(
                        "SELECT effect_id FROM effect_state
                         WHERE state IN ('succeeded', 'terminal_failed')
                           AND settled_at IS NOT NULL AND settled_at < $1
                         LIMIT $2",
                        (cutoff, batch),
                    )
                    .map_err(|e| PartialRetentionFailure { deleted, source: backend_err(e) })?;
                let mut ids = Vec::new();
                for row in rows {
                    let row = row.map_err(|e| PartialRetentionFailure { deleted, source: backend_err(e) })?;
                    ids.push(
                        row.get::<String>(0)
                            .map_err(|e| PartialRetentionFailure { deleted, source: backend_err(e) })?,
                    );
                }
                ids
            };
            delete_batch_preserving_partial_count(effect_ids, &mut deleted, |id| {
                db.execute("DELETE FROM effect_state WHERE effect_id = $1", (id.clone(),))
                    .map_err(backend_err)
            })?;

            let dedup_scopes: Vec<(String, String, String)> = {
                let rows = db
                    .query(
                        "SELECT tenant, effect_type, idempotency_key FROM effect_dedup
                         WHERE succeeded = true
                           AND settled_at IS NOT NULL AND settled_at < $1
                         LIMIT $2",
                        (cutoff, batch),
                    )
                    .map_err(|e| PartialRetentionFailure { deleted, source: backend_err(e) })?;
                let mut scopes = Vec::new();
                for row in rows {
                    let row = row.map_err(|e| PartialRetentionFailure { deleted, source: backend_err(e) })?;
                    scopes.push((
                        row.get::<String>(0)
                            .map_err(|e| PartialRetentionFailure { deleted, source: backend_err(e) })?,
                        row.get::<String>(1)
                            .map_err(|e| PartialRetentionFailure { deleted, source: backend_err(e) })?,
                        row.get::<String>(2)
                            .map_err(|e| PartialRetentionFailure { deleted, source: backend_err(e) })?,
                    ));
                }
                scopes
            };
            delete_batch_preserving_partial_count(
                dedup_scopes,
                &mut deleted,
                |(tenant, effect_type, idempotency_key)| {
                    Self::delete_eligible_dedup_scope(db, tenant, effect_type, idempotency_key, cutoff)
                },
            )?;

            if deleted > 0 {
                log_cleanup_deleted(deleted, "effect_state+effect_dedup");
            }
            Ok(deleted)
        })
        .await
        .unwrap_or_else(|join_err| {
            Err(PartialRetentionFailure {
                deleted: 0,
                source: EffectStoreError::Backend(format!("blocking task panicked: {join_err}")),
            })
        })
    }

    /// Deletes one `effect_dedup` scope row **iff** it is still eligible at
    /// delete time (fix 1, resilience review — AD-8).
    ///
    /// The batch above is built from a `SELECT` snapshot; between that
    /// snapshot and this `DELETE`, the scope's owning effect can be
    /// `release()`d and a new effect can `reserve()` `Fresh` on the exact
    /// same `(tenant, effect_type, idempotency_key)` key — this is ordinary
    /// runtime behavior (e.g. `runner.rs`'s `abandon_and_release` on any
    /// terminal failure), not an edge case. Matching on the scope key alone
    /// would then delete the new, live, non-terminal reservation. Re-checking
    /// `succeeded`/`settled_at < cutoff` here means a scope key that no
    /// longer holds an eligible row survives.
    fn delete_eligible_dedup_scope(
        db: &Database,
        tenant: &str,
        effect_type: &str,
        idempotency_key: &str,
        cutoff: DateTime<Utc>,
    ) -> Result<i64, EffectStoreError> {
        db.execute(
            "DELETE FROM effect_dedup
             WHERE tenant = $1 AND effect_type = $2 AND idempotency_key = $3
               AND succeeded = true AND settled_at IS NOT NULL AND settled_at < $4",
            (
                tenant.to_string(),
                effect_type.to_string(),
                idempotency_key.to_string(),
                cutoff,
            ),
        )
        .map_err(backend_err)
    }

    /// Classifies why a conditional transition to `to` affected zero rows.
    ///
    /// Deliberately returns `Result<EffectStoreError, EffectStoreError>`: the
    /// `Ok` value is the classified transition error to bubble up to the
    /// caller (`NotFound` or `InvalidTransition`), while `Err` is a genuine
    /// backend query failure encountered while classifying. Call sites read
    /// `Err(Self::transition_error(db, id, to)?)` — the `?` unwraps the
    /// *query* failure case, and the resulting `Ok(..)` is then wrapped in
    /// `Err(..)` as the actual error to return to the port caller.
    ///
    /// Takes `&Database` rather than `&self` so it can run inside a
    /// [`run_blocking`](Self::run_blocking) closure alongside the cloned
    /// handle already captured there.
    fn transition_error(
        db: &Database,
        id: EffectId,
        to: EffectState,
    ) -> Result<EffectStoreError, EffectStoreError> {
        let mut rows = db
            .query(
                "SELECT state FROM effect_state WHERE effect_id = $1",
                (id.to_string(),),
            )
            .map_err(backend_err)?;
        match rows.next() {
            None => Ok(EffectStoreError::NotFound(id)),
            Some(row) => {
                let row = row.map_err(backend_err)?;
                let from: String = row.get(0).map_err(backend_err)?;
                Ok(EffectStoreError::InvalidTransition {
                    id,
                    from: state_from_str(&from)?,
                    to,
                })
            }
        }
    }
}

/// Wires the runtime-owned [`RetentionMaintenance`] capability (PROD-002
/// G12) straight through to [`StoolapEffectStore::run_retention`] — no SQL
/// duplicated here. `purge_before` already receives a computed cutoff, so it
/// calls through with a zero `ttl`: `run_retention`'s own `cutoff = now -
/// ttl` then reduces to exactly the cutoff this trait was handed.
#[async_trait]
impl RetentionMaintenance for StoolapEffectStore {
    async fn purge_before(&self, cutoff: Timestamp, batch: usize) -> Result<u64, EffectStoreError> {
        let batch = i64::try_from(batch).unwrap_or(i64::MAX);
        self.run_retention(cutoff, Duration::zero(), batch)
            .await
            .map_err(|failure| failure.source)
    }
}

#[async_trait]
impl EffectStateStore for StoolapEffectStore {
    fn capabilities(&self) -> EffectStoreCapabilities {
        EffectStoreCapabilities {
            durable: true,
            concurrent_local_safe: true,
            multi_node_safe: false,
            supports_leases: false,
        }
    }

    async fn accept(&self, effect: AcceptedEffect) -> Result<(), EffectStoreError> {
        self.run_blocking(move |db| {
            let payload = BASE64.encode(&effect.description.payload);
            let affected = db
                .execute(
                    "INSERT INTO effect_state
                        (effect_id, tenant, effect_type, destination, idempotency_key,
                         payload, attempt, state, next_at, terminal_reason, settled_at)
                     VALUES ($1, $2, $3, $4, $5, $6, $7, 'pending', NULL, NULL, NULL)
                     ON CONFLICT (effect_id) DO NOTHING",
                    (
                        effect.id.to_string(),
                        effect.tenant.as_str().to_string(),
                        effect.description.effect_type.clone(),
                        effect.description.destination.clone(),
                        effect.description.idempotency_key.as_str().to_string(),
                        payload.clone(),
                        effect.attempt as i64,
                    ),
                )
                .map_err(backend_err)?;

            if affected == 1 {
                return Ok(());
            }

            // Already present — classify replay-vs-conflict (AD-9 idempotent accept).
            let mut rows = db
                .query(
                    "SELECT tenant, effect_type, destination, idempotency_key, payload
                     FROM effect_state WHERE effect_id = $1",
                    (effect.id.to_string(),),
                )
                .map_err(backend_err)?;
            let row = rows
                .next()
                .ok_or(EffectStoreError::NotFound(effect.id))?
                .map_err(backend_err)?;
            let existing_tenant: String = row.get(0).map_err(backend_err)?;
            let existing_type: String = row.get(1).map_err(backend_err)?;
            let existing_dest: String = row.get(2).map_err(backend_err)?;
            let existing_key: String = row.get(3).map_err(backend_err)?;
            let existing_payload: String = row.get(4).map_err(backend_err)?;

            if existing_tenant == effect.tenant.as_str()
                && existing_type == effect.description.effect_type
                && existing_dest == effect.description.destination
                && existing_key == effect.description.idempotency_key.as_str()
                && existing_payload == payload
            {
                Ok(())
            } else {
                Err(EffectStoreError::Conflict(format!(
                    "effect {} already accepted with different tenant/description",
                    effect.id
                )))
            }
        })
        .await
    }

    async fn mark_in_flight(&self, id: EffectId) -> Result<(), EffectStoreError> {
        self.run_blocking(move |db| {
            let affected = db
                .execute(
                    "UPDATE effect_state SET state = 'in_flight'
                     WHERE effect_id = $1 AND state IN ('pending', 'retryable_failed')",
                    (id.to_string(),),
                )
                .map_err(backend_err)?;
            if affected == 1 {
                Ok(())
            } else {
                Err(Self::transition_error(db, id, EffectState::InFlight)?)
            }
        })
        .await
    }

    async fn mark_succeeded(&self, id: EffectId) -> Result<(), EffectStoreError> {
        self.run_blocking(move |db| {
            let affected = db
                .execute(
                    "UPDATE effect_state SET state = 'succeeded', settled_at = $2
                     WHERE effect_id = $1 AND state = 'in_flight'",
                    (id.to_string(), Utc::now()),
                )
                .map_err(backend_err)?;
            if affected == 1 {
                Ok(())
            } else {
                Err(Self::transition_error(db, id, EffectState::Succeeded)?)
            }
        })
        .await
    }

    async fn mark_retryable(
        &self,
        id: EffectId,
        attempt: u32,
        next_at: Timestamp,
    ) -> Result<(), EffectStoreError> {
        self.run_blocking(move |db| {
            let affected = db
                .execute(
                    "UPDATE effect_state SET state = 'retryable_failed', attempt = $2, next_at = $3
                     WHERE effect_id = $1 AND state = 'in_flight'",
                    (id.to_string(), attempt as i64, next_at.into_utc()),
                )
                .map_err(backend_err)?;
            if affected == 1 {
                Ok(())
            } else {
                Err(Self::transition_error(db, id, EffectState::RetryableFailed)?)
            }
        })
        .await
    }

    async fn mark_terminal(
        &self,
        id: EffectId,
        reason: TerminalReason,
    ) -> Result<(), EffectStoreError> {
        let encoded = encode_reason(&reason);
        self.run_blocking(move |db| {
            let affected = db
                .execute(
                    "UPDATE effect_state SET state = 'terminal_failed', terminal_reason = $2, settled_at = $3
                     WHERE effect_id = $1 AND state IN ('in_flight', 'retryable_failed')",
                    (id.to_string(), encoded, Utc::now()),
                )
                .map_err(backend_err)?;
            if affected == 1 {
                Ok(())
            } else {
                Err(Self::transition_error(db, id, EffectState::TerminalFailed)?)
            }
        })
        .await
    }

    async fn claim_due(
        &self,
        now: Timestamp,
        limit: usize,
    ) -> Result<Vec<StoredEffect>, EffectStoreError> {
        self.run_blocking(move |db| {
            let rows = db
                .query(
                    "SELECT effect_id, tenant, effect_type, destination, idempotency_key,
                            payload, attempt, state, next_at
                     FROM effect_state
                     WHERE state IN ('pending', 'retryable_failed')
                       AND (next_at IS NULL OR next_at <= $1)
                     ORDER BY next_at
                     LIMIT $2",
                    (now.into_utc(), limit as i64),
                )
                .map_err(backend_err)?;

            let mut out = Vec::new();
            for row in rows {
                let row = row.map_err(backend_err)?;
                let id_str: String = row.get(0).map_err(backend_err)?;
                let tenant: String = row.get(1).map_err(backend_err)?;
                let effect_type: String = row.get(2).map_err(backend_err)?;
                let destination: String = row.get(3).map_err(backend_err)?;
                let idempotency_key: String = row.get(4).map_err(backend_err)?;
                let payload_b64: String = row.get(5).map_err(backend_err)?;
                let attempt: i64 = row.get(6).map_err(backend_err)?;
                let state: String = row.get(7).map_err(backend_err)?;
                let next_at = match row.get_value(8) {
                    Some(stoolap::Value::Timestamp(dt)) => Timestamp::from_utc(*dt),
                    _ => now,
                };

                let payload = BASE64
                    .decode(payload_b64.as_bytes())
                    .map_err(|e| EffectStoreError::Backend(format!("payload decode: {e}")))?;

                out.push(StoredEffect {
                    id: EffectId::from_uuid(
                        id_str
                            .parse()
                            .map_err(|e: uuid::Error| EffectStoreError::Backend(format!("effect_id decode: {e}")))?,
                    ),
                    tenant: TenantId::new(tenant)
                        .map_err(|e| EffectStoreError::Backend(format!("tenant decode: {e}")))?,
                    description: Arc::new(ExternalEffectDescription {
                        idempotency_key: IdempotencyKey::new(idempotency_key).map_err(|e| {
                            EffectStoreError::Backend(format!("idempotency_key decode: {e}"))
                        })?,
                        effect_type,
                        payload,
                        destination,
                    }),
                    attempt: attempt as u32,
                    state: state_from_str(&state)?,
                    next_at,
                });
            }
            Ok(out)
        })
        .await
    }

    async fn recover_in_flight(&self, now: Timestamp) -> Result<u64, EffectStoreError> {
        self.run_blocking(move |db| {
            let affected = db
                .execute(
                    "UPDATE effect_state SET state = 'pending', next_at = $1 WHERE state = 'in_flight'",
                    (now.into_utc(),),
                )
                .map_err(backend_err)?;
            Ok(affected.max(0) as u64)
        })
        .await
    }
}

#[async_trait]
impl EffectDedupStore for StoolapEffectStore {
    fn capabilities(&self) -> EffectStoreCapabilities {
        EffectStoreCapabilities {
            durable: true,
            concurrent_local_safe: true,
            multi_node_safe: false,
            supports_leases: false,
        }
    }

    async fn reserve(
        &self,
        scope: &DedupScope,
        effect_id: EffectId,
        fingerprint: EffectFingerprint,
    ) -> Result<DedupOutcome, EffectStoreError> {
        let tenant = scope.tenant.as_str().to_string();
        let effect_type = scope.effect_type.clone();
        let key = scope.key.as_str().to_string();
        let fp_hex = hex_encode(fingerprint.as_bytes());

        self.run_blocking(move |db| {
            let affected = db
                .execute(
                    "INSERT INTO effect_dedup
                        (tenant, effect_type, idempotency_key, effect_id, fingerprint, succeeded, settled_at)
                     VALUES ($1, $2, $3, $4, $5, false, NULL)
                     ON CONFLICT (tenant, effect_type, idempotency_key) DO NOTHING",
                    (
                        tenant.clone(),
                        effect_type.clone(),
                        key.clone(),
                        effect_id.to_string(),
                        fp_hex.clone(),
                    ),
                )
                .map_err(backend_err)?;

            if affected == 1 {
                return Ok(DedupOutcome::Fresh);
            }

            let mut rows = db
                .query(
                    "SELECT effect_id, fingerprint, succeeded FROM effect_dedup
                     WHERE tenant = $1 AND effect_type = $2 AND idempotency_key = $3",
                    (tenant, effect_type, key),
                )
                .map_err(backend_err)?;
            let row = rows
                .next()
                .ok_or_else(dedup_row_vanished_error)?
                .map_err(backend_err)?;
            let existing_owner: String = row.get(0).map_err(backend_err)?;
            let existing_fp: String = row.get(1).map_err(backend_err)?;
            let succeeded: bool = row.get(2).map_err(backend_err)?;

            if existing_fp != fp_hex {
                return Ok(DedupOutcome::Conflict);
            }
            if existing_owner == effect_id.to_string() {
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
        })
        .await
    }

    async fn commit_success(&self, scope: &DedupScope) -> Result<(), EffectStoreError> {
        let tenant = scope.tenant.as_str().to_string();
        let effect_type = scope.effect_type.clone();
        let key = scope.key.as_str().to_string();

        self.run_blocking(move |db| {
            db.execute(
                "UPDATE effect_dedup SET succeeded = true, settled_at = $4
                 WHERE tenant = $1 AND effect_type = $2 AND idempotency_key = $3",
                (tenant, effect_type, key, Utc::now()),
            )
            .map_err(backend_err)?;
            Ok(())
        })
        .await
    }

    async fn release(&self, scope: &DedupScope) -> Result<(), EffectStoreError> {
        let tenant = scope.tenant.as_str().to_string();
        let effect_type = scope.effect_type.clone();
        let key = scope.key.as_str().to_string();

        self.run_blocking(move |db| {
            db.execute(
                "DELETE FROM effect_dedup
                 WHERE tenant = $1 AND effect_type = $2 AND idempotency_key = $3",
                (tenant, effect_type, key),
            )
            .map_err(backend_err)?;
            Ok(())
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn fresh_store() -> StoolapEffectStore {
        let dir = tempfile::tempdir().expect("tempdir");
        StoolapEffectStore::open(dir.path())
            .await
            .expect("open StoolapEffectStore")
    }

    // --- Fix 2: backend_err / dedup_row_vanished_error classification ---

    #[test]
    fn backend_err_classifies_lock_failures_as_temporarily_unavailable() {
        let err = backend_err(stoolap::Error::LockAcquisitionFailed("held by another writer".into()));
        assert!(
            matches!(err, EffectStoreError::TemporarilyUnavailable(_)),
            "LockAcquisitionFailed must be retryable, got {err:?}"
        );

        let err = backend_err(stoolap::Error::DatabaseLocked);
        assert!(
            matches!(err, EffectStoreError::TemporarilyUnavailable(_)),
            "DatabaseLocked must be retryable, got {err:?}"
        );
    }

    #[test]
    fn backend_err_classifies_other_stoolap_errors_as_backend() {
        let err = backend_err(stoolap::Error::TableNotFound("effect_state".into()));
        assert!(
            matches!(err, EffectStoreError::Backend(_)),
            "a non-transient Stoolap error must stay Backend, got {err:?}"
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

    // --- Fix 3: an unrecognized persisted state must not panic ---

    /// Isolated unit test on `state_from_str` itself — deliberately does
    /// NOT go through `run_blocking`/`spawn_blocking`, whose `JoinError`
    /// handling would otherwise convert even a genuine panic into a
    /// `Backend` error and mask whether *this function* still panics.
    #[test]
    fn state_from_str_rejects_unknown_value_without_panicking() {
        let err = state_from_str("bogus_state").unwrap_err();
        assert!(
            matches!(err, EffectStoreError::Backend(_)),
            "an unrecognized persisted state must be a classified Backend error, not a panic, got {err:?}"
        );
    }

    #[tokio::test]
    async fn unknown_persisted_state_returns_backend_error_not_panic() {
        let store = fresh_store().await;
        let id = EffectId::new();

        // Bypass the trait entirely: insert a row with a state value this
        // provider never writes itself (legacy/cross-version/corrupt data).
        store
            .db
            .execute(
                "INSERT INTO effect_state
                    (effect_id, tenant, effect_type, destination, idempotency_key,
                     payload, attempt, state, next_at, terminal_reason, settled_at)
                 VALUES ($1, 'tenant-a', 'invoice.created', 'https://example.com',
                         'uow-bogus:0', '', 0, 'bogus_state', NULL, NULL, NULL)",
                (id.to_string(),),
            )
            .expect("insert malformed row");

        let err = store.mark_in_flight(id).await.unwrap_err();
        assert!(
            matches!(err, EffectStoreError::Backend(_)),
            "an unrecognized persisted state must be a classified Backend error, not a panic, got {err:?}"
        );
    }

    // --- Fix 5: partial delete count preserved on mid-batch failure ---

    #[test]
    fn delete_batch_preserving_partial_count_keeps_prior_successes_on_failure() {
        let items = vec![1, 2, 3, 4];
        let mut deleted = 0u64;
        let mut calls = 0u32;

        let result = delete_batch_preserving_partial_count(items, &mut deleted, |_| {
            calls += 1;
            if calls <= 2 {
                Ok(1)
            } else {
                Err(EffectStoreError::Backend("boom".into()))
            }
        });

        let err = result.unwrap_err();
        assert_eq!(err.deleted, 2, "the two rows deleted before the failure must not be lost");
        assert_eq!(
            deleted, 2,
            "the caller's running total must also reflect the two successful deletes"
        );
        assert!(matches!(err.source, EffectStoreError::Backend(_)));
    }

    // --- Fix 1: dedup retention TOCTOU ---

    #[tokio::test]
    async fn delete_eligible_dedup_scope_does_not_delete_a_live_non_terminal_reservation() {
        let store = fresh_store().await;
        let s = DedupScope {
            tenant: TenantId::new("tenant-a").unwrap(),
            effect_type: "invoice.created".to_string(),
            key: IdempotencyKey::new("toctou-uow:0").unwrap(),
        };
        let old_owner = EffectId::new();
        let fp = EffectFingerprint::compute(b"payload", "https://example.com");

        // Old reservation settles (succeeded) — eligible for retention.
        assert_eq!(store.reserve(&s, old_owner, fp).await.unwrap(), DedupOutcome::Fresh);
        store.commit_success(&s).await.unwrap();
        let cutoff = Utc::now();

        // Race window: released, then a fresh reservation lands on the SAME
        // scope key before the delete executes.
        store.release(&s).await.unwrap();
        let new_owner = EffectId::new();
        assert_eq!(store.reserve(&s, new_owner, fp).await.unwrap(), DedupOutcome::Fresh);

        // The fixed predicate must reject the delete: the row now sharing
        // this scope key is no longer succeeded/settled-before-cutoff.
        let affected = StoolapEffectStore::delete_eligible_dedup_scope(
            &store.db,
            s.tenant.as_str(),
            &s.effect_type,
            s.key.as_str(),
            cutoff,
        )
        .unwrap();
        assert_eq!(
            affected, 0,
            "a live, non-terminal reservation sharing the scope key must survive the delete"
        );

        // And the new reservation is provably intact, not silently gone.
        assert_eq!(
            store.reserve(&s, new_owner, fp).await.unwrap(),
            DedupOutcome::OwnedInProgress,
            "the fresh reservation must still be observable after the rejected delete"
        );
    }
}
