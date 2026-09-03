## Exploration: PROD-014C — Atomic Read-Side Event Claiming

### Current State — real flow reconstruction

**Entry point**: `crates/domain/src/read_side/session.rs` `ReadSideSession::execute()` (lines 91-176) is the ONLY place a batch is processed. Five sequential, unguarded async calls, no shared transaction, no lock:

1. `offset_store.read_offset(projection_id, tag, tenant)` (line 92-96) — reads current position fresh EVERY call (not cached in memory across polls).
2. `read_store.fetch(tenant, tag, last_offset, batch_size)` (line 101-110) — returns events with `event_version > offset` in ascending order (`crates/persistence-api/src/read_side/store.rs:30`, `ReadSideStore::fetch` doc contract). This is the per-`(tenant, tag)` ordering guarantee.
3. Per-event dedup filter: `dedup_store.seen(projection_id, tag, event_id)` in a loop (lines 118-128) — builds `unique_events`.
4. `handler.handle(&unique_events)` (line 135) — the ONE call to arbitrary user code; `Handler<E>: Send + Sync` (`crates/domain/src/read_side/handler.rs`) has no restriction — it CAN perform external I/O. Doc comment only says "Implementations should be idempotent where possible" — not enforced anywhere.
5. On `Ok(())`: `dedup_store.mark_seen(...)` per event, sequentially (lines 143-150), THEN `offset_store.write_offset(...)` once (lines 152-158).

Caller chain: `crates/runtime/src/read_side/scheduler.rs` `TagSchedulerImpl::start_projection` builds one `ReadSideSession` per `(tag, tenant)` pair and calls `self.batch_executor.execute_session(session).await` **sequentially in a for-loop** (not concurrently spawned) — `crates/runtime/src/read_side/batch_executor.rs::execute_session` only acquires a `Backpressure` semaphore permit, no transaction/lock. `TagSchedulerImpl::spawn` (line 276) wraps `start_projection` in a `tokio::spawn`'d poll loop (default interval 1s), one loop per process. Nothing coordinates across processes/replicas — each replica runs its own independent loop against the same Postgres tables.

### Race window — exact location

Between step 3 (`dedup_store.seen()` returning `false`) and step 5 (`dedup_store.mark_seen()`), two concurrent replicas processing the same `(projection_id, tag, tenant)` can both read `seen()=false` for the same event, both run `handler.handle()`, and both then call `mark_seen()` — the second one's `INSERT ... ON CONFLICT DO NOTHING` (`crates/persistence/src/postgres/read_side_dedup.rs:109-119`) makes the ROW converge to one, but does not undo the second `handler.handle()` invocation that already ran. `write_offset` similarly is a plain upsert (`crates/persistence/src/postgres/read_side_offset.rs:87-109`) with "no compare-and-swap, no expected-previous-offset check" (own doc comment, lines 5-14) — a slow writer can overwrite a fast writer's more-advanced offset with a stale one.

### Transactional guarantees — none, and why

`ReadSideSession<E,H,RS,DS,OS,PR>` is generic over trait objects (`DS: DedupStore`, `OS: OffsetStore`) with no `begin()`/transaction handle threaded through — the domain crate (`crates/domain`) has zero PostgreSQL awareness by hexagonal design; only `crates/persistence` knows about `sqlx::PgPool`. Even the two Postgres adapters, which DO both use `sqlx`, never wrap `mark_seen`+`write_offset` (across the two SEPARATE tables/structs `PostgreSQLDedupStore`/`PostgreSQLOffsetStore`) in one transaction — each does its own `.execute(&self.pool)` (auto-commit). So even the "storage convergence" bookkeeping across offset+dedup together is not atomic, only each individual upsert is atomic within its own table.

### Explicit self-documentation already in the codebase (strong evidence, not invented)

`crates/persistence/src/postgres/read_side_dedup.rs:1-25` doc comment states verbatim: "Two concurrent `mark_seen` calls ... converge to one row ... It does not prevent a handler from having already run twice ... This capability delivers at-least-once handler execution with best-effort dedup bookkeeping, never exactly-once handling." Same file and `read_side_offset.rs:1-14` both name "exactly one writer per `(projection_id, tag, tenant)`" as an "external, unenforced adoption constraint" and explicitly name PROD-014C as the follow-up that would close it. `ARCHITECTURE.md:211-219` and `examples/reference-app/src/read_side/mod.rs:118-126` (`ReadSideProgressStores::postgres` doc comment) repeat the identical claim boundary and follow-up naming.

Verified directly (not just via the sub-agent report):
```
crates/persistence/src/postgres/read_side_offset.rs:13://! refuses that configuration — see **PROD-014C — Atomic Read-Side Event
ARCHITECTURE.md:218:gap is **PROD-014C — Atomic Read-Side Event Claiming**, a named, distinct follow-up, not
examples/reference-app/src/read_side/mod.rs:126:    /// it — see PROD-014C — Atomic Read-Side Event Claiming.
crates/persistence/src/postgres/read_side_dedup.rs:17://! constraint across replicas — see **PROD-014C — Atomic Read-Side Event
```

### Failure-mode walkthrough (current code, traced not assumed)

- **Dies before handler** (between dedup filter and `handler.handle()`): dedup not yet marked, offset not written → next poll (this replica after restart, or ANY other replica already running) re-fetches, sees not-seen, re-runs handler once. Safe — no anomaly beyond normal at-least-once.
- **Dies during handler**: handler may have partially executed side effects with unknown outcome; dedup/offset both still unwritten → guaranteed re-run on next poll of the SAME event. If the handler performed an external, non-idempotent effect, that effect can double-fire — this is true even with a single replica and no concurrency at all, since crash-recovery alone forces at-least-once handler execution.
- **Dies after handler succeeds, before `mark_seen`**: identical outcome to above — handler re-runs on resume even without any second worker.
- **Dies mid-`mark_seen` loop** (some events in the batch marked, others not) **before `write_offset`**: offset stays at the pre-batch value, so the ENTIRE original batch is re-fetched on resume; the dedup filter (step 3) now strips out the already-marked events, so the handler is re-invoked with a SMALLER/different batch than the original — a correctness nuance if any handler assumed batch completeness/atomicity across a call.
- **Loses PG connectivity**: `is_fatal()` classifies most connection errors as `Transient` (`crate::postgres::is_fatal`), surfacing `ProjectionError::Transient` from `session.execute()`. The scheduler's `on_error` callback (logging only) fires and the loop simply waits for the next poll interval — **the "Retry batch with exponential backoff (max 3 retries, 100ms base, 10s max)" documented in `crates/domain/src/read_side/error.rs:9` is NOT implemented anywhere in `scheduler.rs`/`session.rs`/`batch_executor.rs`** (confirmed by grep for "backoff"/"retry" across both crates — zero hits outside that one doc comment). This is a pre-existing, separate gap from the claiming problem — worth flagging to `sdd-propose` but must not be conflated with PROD-014C's scope.
- **Pauses for a long time then resumes after another worker took over**: read-side has NO lease/heartbeat/ownership row at all (unlike the entity-runtime's `operation_reservations`, see below), so "took over" is not a modeled concept here. Because `read_offset` is re-read fresh from Postgres on every single `execute()` call (never cached across polls — the domain-layer comment at `session.rs:81-87` states this explicitly), a resumed-after-pause worker does NOT risk writing a stale offset — but it is exactly as exposed to the seen()/mark_seen() race as a normally-running concurrent replica; a long pause doesn't create a new risk class, it just widens the same one's time window.

### Can handlers produce side effects outside PostgreSQL? (answers whether exactly-once processing is even possible in principle)

Yes, in principle. `Handler<E>: Send + Sync { async fn handle(&self, events) -> Result<(), ProjectionError> }` (`crates/domain/src/read_side/handler.rs`) places zero restriction on what a handler does — it is arbitrary async code. The workspace's only real production handler today, `UsersByTenantHandler` (`examples/reference-app/src/read_side/projection.rs`, wired in `examples/reference-app/src/read_side/mod.rs`), only writes to an in-process `UsersByTenantStore` (a query-model cache) — it performs no external I/O today. But nothing in the port, the runtime, or the framework prevents a future handler from calling an external API. This means: even a perfect atomic-claim mechanism over the dedup/offset bookkeeping can only ever guarantee exactly-once EXECUTION of the handler call itself (a framework-level property) — it structurally cannot guarantee exactly-once EXTERNAL SIDE EFFECTS unless the handler's own effect boundary is separately idempotent/fenced (mirrors the exact same caveat already documented for the write-side reservation store at `crates/persistence/src/postgres/reservation.rs:22-26`: "It guarantees nothing about an external effect already in flight ... Avoiding a duplicated external effect requires the effect boundary itself to carry the fence or to be idempotent on its own"). This is out of scope per the task's own non-goals list ("exactly-once external side effects") — confirmed as architecturally correct to exclude, not just asserted.

### Existing single-writer / claim / lease / fencing patterns in the codebase (exhaustive grep, point 6)

**None exist for the read-side.** Grep for "single writer|single-writer|one worker|advisory lock|SKIP LOCKED|FOR UPDATE|lease|fencing|claim" across the whole workspace returns 250+ files, but every hit relevant to read-side processing is a DOC COMMENT describing the absence of such a mechanism (the PROD-014C follow-up references above) — zero executable code.

**A directly analogous, battle-tested mechanism DOES exist, but for a different domain — the WRITE side (entity-runtime command idempotency, PROD-012/PROD-012A), not read-side event processing:**

- `crates/domain/src/operation/reservation.rs` defines `OperationReservationStore` (`reserve`/`renew`/`complete`/`abandon`/`purge_completed_before`/`oldest_completed`/`probe`), with `FencingToken`, `Lease`, `OwnerId`, `OwnerFence` types.
- `crates/persistence/src/postgres/reservation.rs` implements it: `reserve()` does `INSERT ... ON CONFLICT DO NOTHING` for a fresh key (lines 213-228), falls through to a conditional takeover `UPDATE ... WHERE state='in_progress' AND fencing_token=$N AND lease_until<=$N` that atomically re-owns the row AND mints a strictly-greater fencing token in one statement (lines 313-340) when the existing lease has expired (compared against an injected `Clock`, never DB `now()` — AD-8). `renew`/`complete`/`abandon` all use a shared `mutate_owned` helper (lines 578-605) that verifies the full `(tenant, key, owner, fencing_token, lease_until > now)` tuple in one `WHERE` clause per statement — no separate check-then-write window. `purge_completed_before` uses `FOR UPDATE SKIP LOCKED` for progress (not safety) under concurrent workers (lines 468-501).
- This mechanism is proven under real concurrent-replica load by `integration-tests/tests/infrastructure/{concurrent_replicas_postgres,lease_contention_postgres,fencing_window_postgres,takeover_fencing_postgres}.rs` — six real contenders racing one row lock, real HTTP → real Postgres, asserting "exactly one execution" end to end.
- **This is NOT the read-side's stop-condition mechanism** — it governs `operation_reservations` (client-supplied idempotency keys on entity commands), has zero references to `OffsetStore`/`DedupStore`/`ReadSideSession`/projections, and cannot be reused unmodified (different identity shape: `(tenant, operation_key)` vs. read-side's `(projection_id, tag, tenant)`; different lifecycle: one-shot command vs. continuous poll loop). It IS, however, strong precedent for the SQL shape (conditional UPDATE with fencing-token CAS, `ON CONFLICT DO NOTHING` for fresh claims, injected-clock expiry) that a read-side claim design could mirror — this is evidence for `sdd-propose` to weigh, not a design decision made here.

### STOP CONDITION CHECK — explicitly evaluated, does NOT trigger

The codebase does NOT already have an atomic-claim/execution-exclusion guarantee for read-side event processing. The `operation_reservations` mechanism above is real, atomic, and lease/fencing-based, but it protects a structurally different resource (entity-command idempotency keys), not read-side projection event claiming. **PROD-014C's premise is confirmed valid — proceed to `sdd-propose`.**

### Claim granularity — evidence, not assumption

The natural granularity is **`(projection_id, tag, tenant)`**, matching the OffsetStore's own documented identity: "Offsets are independent per (projection_id, tag, tenant) tuple" (`crates/persistence-api/src/read_side/offset.rs:53`) and the `projection_offsets` table's own primary key (`crates/persistence/src/postgres/migrations/013_create_projection_offsets.sql:24-25`: `PRIMARY KEY (projection_id, tag, tenant)`). This triple is the unit that owns one monotonically-advancing offset and therefore the unit whose processing must be serialized to prevent the race — NOT per-event (finer than needed, since ordering must already be preserved per-tag/tenant, so per-event claiming inside one stream buys nothing extra) and NOT per-partition (no separate partition concept exists in this codebase beyond `tag`; `tag` in the reference app already encodes a per-tenant partition via `tenant_tag()`, e.g. `"users-by-tenant:tenant-a"`). Dedup identity is a DIFFERENT, narrower triple — `(projection_id, tag, event_id)`, no tenant column (`projection_dedup` PK, migration 014) — by deliberate design (AD-7 in the PROD-014B design), since dedup tracks event presence, not tenant-owned state.

### Ordering guarantees today (so a design doesn't accidentally break it)

`ReadSideStore::fetch` contract (`crates/persistence-api/src/read_side/store.rs:30`): "Returns events with `event_version > offset` in ascending version order." This is a per-`(tenant, tag)` stream ordering guarantee, established at the store boundary, unrelated to and unaffected by claiming/dedup. Any claiming design MUST preserve strict per-`(tag, tenant)` sequential handler invocation — claiming at a coarser granularity than `(projection_id, tag, tenant)` (e.g., a global claim, or claiming individual events within one stream out of order) would risk violating this existing ordering promise; claiming at exactly this granularity (one claim per stream, held for the duration of one batch) preserves it trivially since only the claim holder ever calls `fetch`/`handle` for that stream.

### Affected Areas
- `crates/domain/src/read_side/session.rs` — `ReadSideSession::execute()`, the exact 5-call unguarded sequence.
- `crates/runtime/src/read_side/scheduler.rs` — `TagSchedulerImpl::start_projection`/`spawn`, the per-process poll loop with no cross-process coordination.
- `crates/persistence-api/src/read_side/{offset,dedup}.rs` — trait definitions; a claim primitive would likely need a NEW port (not bolted onto `OffsetStore`/`DedupStore`, whose contracts are already shipped/tested/archived under PROD-014B) — `sdd-propose`'s call.
- `crates/persistence/src/postgres/{read_side_offset,read_side_dedup}.rs` — durable adapters whose own doc comments already name PROD-014C as the gap-closer; likely need a companion adapter or migration, not a rewrite.
- `crates/persistence/src/postgres/reservation.rs` — reusable SQL-pattern precedent (conditional UPDATE + fencing CAS), not code to import directly.
- `examples/reference-app/src/read_side/mod.rs` — `ReadSideProgressStores::postgres()` doc comment (lines 118-126) explicitly promises this gap will close here; likely needs updating once a claim mechanism exists.

### Approaches (for `sdd-propose` to weigh, not decided here)

| Approach | Pros | Cons | Effort |
|---|---|---|---|
| PG row lock (`FOR UPDATE`) on a per-stream row | Simple, reuses Postgres primitives | Needs a row to lock — likely still needs a new claim table | Low-Med |
| PG advisory lock keyed by `(projection_id, tag, tenant)` hash | No new table | Session-scoped semantics awkward for a poll-loop-per-tick model; no visibility/audit trail | Low-Med |
| New durable claim table (`try_claim`/`renew`/`complete`/`release`), mirroring `operation_reservations`' conditional-UPDATE+fencing shape | Precedented, testable the same way, supports lease+takeover | New port + migration + design delta against shipped `read-side` spec | Med-High |
| `FOR UPDATE SKIP LOCKED` work-queue pattern | Good for competing-consumers/queue models | This is a continuous poll-loop-per-stream model, not a queue — fit is questionable, flag as open question | Med |

### Recommendation

Proceed to `sdd-propose`. The strongest lead is a new claim table shaped like `operation_reservations` (lease + fencing token, conditional UPDATE) but keyed on `(projection_id, tag, tenant)`, as a new port rather than bolting onto the already-shipped `OffsetStore`/`DedupStore` contracts. Open question for the architect: new `ClaimStore` port vs. evolving `OffsetStore` (a MODIFIED-requirement spec delta either way, since the PROD-014A/B contracts are shipped/archived).

### Risks

1. Retry-with-backoff for `Transient` `ProjectionError` is documented (`crates/domain/src/read_side/error.rs:9`) but NOT implemented anywhere in the read-side runtime — a pre-existing, separate gap from event claiming; flag to `sdd-propose` as adjacent but out of THIS change's atomic scope.
2. A crash mid-`mark_seen`-loop (before `write_offset`) causes the handler to be re-invoked with a partial/different-shaped batch on resume, not the exact original batch — any future claim design must not assume batch-level atomicity is solved by claiming alone; claiming addresses cross-replica execution overlap, not within-replica crash/batch-partial-completion behavior, which is a different (already-existing, undocumented-as-a-gap) risk.
3. Dedup and offset bookkeeping are not atomic with EACH OTHER even today (separate upserts, no shared transaction) — a claim design that only wraps the seen/handle/mark_seen window but not offset writing would leave this pre-existing gap open.
4. Handler side effects can in principle include external I/O the framework has zero visibility into — any claim mechanism can only bound handler-EXECUTION-count, never external-effect-count, without the handler's own effect boundary separately carrying a fence (exact same caveat already true and documented for the unrelated write-side reservation mechanism).
5. `TagSchedulerImpl::start_projection` processes tags SEQUENTIALLY within one process despite its doc comment saying "respecting concurrency limits" (implies parallelism) — `Backpressure::acquire()` is awaited inline, not used to gate `tokio::spawn`'d concurrent tasks. Worth confirming with `sdd-propose`/design whether a claim design should also address (or deliberately not address) enabling real intra-process concurrency, since claiming makes cross-process safety possible but the current code doesn't yet exploit cross-tag intra-process parallelism at all.

### Ready for Proposal

Yes. The stop condition was checked explicitly and does not apply — no existing mechanism provides atomic read-side event claiming or execution exclusion. `sdd-propose` should compare (at minimum): PG row-level locks (`FOR UPDATE`), PG advisory locks, a new atomic durable claim table (try_claim/renew/complete/release, likely mirroring `operation_reservations`'s conditional-UPDATE+fencing-token shape but keyed on `(projection_id, tag, tenant)` instead of `(tenant, operation_key)`), and whether `FOR UPDATE SKIP LOCKED` work-queue semantics fit a continuous poll-loop model (as opposed to `operation_reservations`' one-shot-per-key model). Open question to flag explicitly to `sdd-propose`: whether a claim mechanism should be a NEW port (`ClaimStore`?) distinct from `OffsetStore`/`DedupStore`, or fold into an evolved `OffsetStore` (the PROD-014A/B contracts are shipped/archived, so extending them is itself a MODIFIED-requirement delta against `openspec/specs/read-side/spec.md`, not just new code).
