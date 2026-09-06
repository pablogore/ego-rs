# Design: STOOLAP-S3 — Operation Reservation Store: Durability Signal, Stoolap Implementation, Production Gate

> Canonical / English. Spanish companion: `design.es.md` (1:1 headings and decision IDs).

## Technical Approach

Three independently revertible slices, in the order the proposal names them.

- **(a)** `OperationReservationStore` gains `fn is_durable(&self) -> bool { false }` plus an
  `Arc<T>` forwarding impl, copied from `read_side/dedup.rs:33-35,59-67`. Implementor sweep:
  in-memory inherits `false`, Postgres overrides `true`.
- **(b)** `StoolapOperationReservationStore` — a new module in `ego-persistence-stoolap`, shaped
  after `StoolapEffectStore` (`spawn_blocking` over the synchronous `Database`, fail-closed
  `open()` requiring `sync=full`) and after `PostgresOperationReservationStore` for the
  reservation algorithm itself (that is the reference implementation of this port).
- **(c)** `validate_operation_reservation_profile()` on `RuntimeBuilder`, sequenced into
  `validate_persistence_profile()` alongside the four existing gates, routing through the one
  shared `persistent_entity::profile::require_durably_configured` predicate.

Specs: `persistence-api-surface` (a), `persistence-memory-adapter` (a),
`idempotent-command-processing` (a), `persistence-stoolap-operation-reservation` (b),
`production-composition-hardening` (c).

## Architecture Decisions

### AD-1: The durability signal is `is_durable()`, and the sibling pattern is not uniform — the variation is documented, not copied blindly

**Choice**: `fn is_durable(&self) -> bool { false }` on the trait, plus
`impl<T: OperationReservationStore + Send + Sync + ?Sized> OperationReservationStore for Arc<T>`
forwarding every method including `is_durable`.

**Alternatives considered**: `effect-store`'s `capabilities() -> EffectStoreCapabilities` struct;
a required (non-defaulted) method.

**Rationale**: The `capabilities()` struct belongs to a different port family in a different
crate (`ego-effect-store`); introducing it here would be the "second way to express durability"
the proposal puts out of scope. The `false` default is what keeps every external implementor
compiling and honestly classified.

**Verified sibling survey** (the pattern is *not* identical across all five — record this):

| Port | `is_durable` default | `Arc<T>` forwarding impl | Note |
|---|---|---|---|
| `read_side/dedup.rs:33` | `false` | yes, `:59-67` | full pattern |
| `read_side/offset.rs:62` | `false` | yes, `:92-99` | full pattern |
| `read_side/claim.rs:73` | `false` | yes, `:117-124` | full pattern |
| `persistence/snapshot.rs:19` | `false` | **no** | held as `Arc<Mutex<dyn Snapshot>>`, not `Arc<dyn _>` |
| `persistence/event_store.rs:54` | `false` | **no** | held as `Arc<dyn EventStore<E>>`, never generically by value |

`OperationReservationStore` is held as `Arc<dyn OperationReservationStore>`
(`builder.rs:121`), so it belongs with the first three: ship the forwarding impl.

### AD-2: The `Arc` impl is required for parity and generic use — **not** because the gate's own call site would regress

**Choice**: Ship the impl; write its test against a generic `S: OperationReservationStore`
instantiated with `Arc<ConcreteStore>`, **not** against the builder gate.

**Rationale**: This corrects an assumption inherited from the proposal. The gate reads
`Option<Arc<dyn OperationReservationStore>>`; on `Arc<dyn Trait>`, `is_durable()` resolves to the
concrete store's override *either way* — via the new impl's forwarding body, or via autoderef to
`&dyn Trait` without it. A test that wraps a durable store in `Arc<dyn _>`, registers it, and
asserts the gate accepts would therefore **pass even if the forwarding impl were deleted** —
vacuous. The impl is genuinely load-bearing where `Arc<Concrete>` must itself *satisfy* the
trait: `assert_reservation_store_conformance<S: OperationReservationStore>`
(`testkit/src/reservation_conformance.rs:963-968`) and any `Arc<Arc<dyn _>>` nesting. Without
it, that generic instantiation does not compile; with a forwarding impl that omits
`is_durable`, it compiles and silently reports `false`. That is the regression the test must
pin.

### AD-3: The Production gate fires on **registration**, not on `IdempotencyEnforcementMode::MandatoryKey`

**Choice**: `validate_operation_reservation_profile()` checks *iff* a reservation store is
registered; a registered store under `Profile::Production` MUST be durable. Absence of a store
is **not** a Production failure and is **not** re-checked here.

**Alternatives considered**: (i) trigger on `MandatoryKey`, mirroring
`validate_effect_store_profile`'s "executors registered ⇒ effect store must be durable";
(ii) `is_some_and(|s| s.is_durable())`, mirroring `validate_read_side_claim_profile`, which makes
absence itself a failure.

**Rationale — which existing convention this is, read from the code, not assumed**: The two
in-tree framings both reduce to one rule: *the gate fires when the composition's own
configuration shows the capability is actually exercised.*

- `validate_effect_store_profile` (`builder.rs:877-896`) is conditional on registered executors
  because "with none registered no effect store is constructed at all".
- `validate_read_side_progress_profile` (`:905-917`) is triggered by registration itself —
  "registration is itself the composition-visible signal that this projection has a progress
  pair worth governing".
- `validate_read_side_claim_profile` (`:927-945`) is triggered by progress registration because a
  command-only service never claims.

For this port the code already answers which signal applies, and it answers it about *this exact
store*. `build()` constructs the `ReservationConfig` from
`self.idempotency_reservation_store` **unconditionally of the mode** (`:1123-1151`), and the
health-contributor decision immediately above it states the rule in as many words
(`:1099-1106`): *"Keyed on the store being present, not on the enforcement mode. A
`Compatibility` runtime that did register one is still dispatching through it, so it is still a
real dependency and is checked."* A registered store is exercised; therefore registration is the
trigger, exactly as in `validate_read_side_progress_profile`. Alternative (i) would leave a real
hole: `Compatibility` + `Production` + volatile store is a composition that reserves through
volatile storage and would be silently accepted.

Alternative (ii) is rejected because `validate_idempotency` (`:846-855`) already owns
"`MandatoryKey` ⇒ a store MUST be registered" and runs first from both `build()` and
`try_build()`. Making absence fail here too would create the second, parallel definition of one
rule that PROD-014A exists to avoid.

**Answer to the open question, stated for `sdd-verify`**: the gate fires when a reservation store
is registered at all. It does **not** additionally require `MandatoryKey`.

### AD-4: One conditional statement per state transition — no multi-statement transaction spans a read and its dependent write

**Choice**: Every mutation is a single conditional SQL statement whose `WHERE` carries the whole
verification. Stoolap executes a bare `Database::execute` as its own atomic unit (the shape
`StoolapEffectStore` uses throughout and proves under its concurrency conformance). `db.begin()`
is used **nowhere** in this store.

**Rationale**: The atomicity unit that closes a check-then-act race here is the predicate, not a
transaction — this is exactly why `PostgresOperationReservationStore::mutate_owned`
(`persistence/src/postgres/reservation.rs:565-605`) needs no transaction either: *"Both live in
the `WHERE` clause, so verification and mutation are one statement: a separate read-then-write
would leave a window."* `StoolapSnapshotStore::save_snapshot` opens a transaction only because it
must do SELECT-then-(INSERT-or-UPDATE) — two statements that must be one unit. No transition
here has that shape.

**Explicitly rejected and to be flagged in review**: any sequence of
`SELECT` → compare in Rust → unconditional `UPDATE`/`DELETE`. Every `reserve` read below is used
only to *classify* and to *compute* the next token; the following write re-asserts every value it
read inside its own `WHERE`, so a stale read cannot produce a wrong write — it produces
`affected == 0`, which is then re-read or reported.

**Per-method transition contract** (this is the literal implementation contract):

| Method | The one atomic statement | Race outcome |
|---|---|---|
| `reserve` / fresh | `INSERT … VALUES (…, 'in_progress') ON CONFLICT (tenant_id, operation_key) DO NOTHING` | `affected==1` ⇒ `Fresh`; `0` ⇒ fall through to classify |
| `reserve` / takeover | `UPDATE … SET owner_id=$, fencing_token=$next, lease_until=$ WHERE tenant_id=$ AND operation_key=$ AND state='in_progress' AND fencing_token=$displaced AND lease_until <= $now` | `affected==1` ⇒ `TakenOver`; `0` ⇒ re-`SELECT` ⇒ `OwnedInProgress` / `OtherInProgress` |
| `renew` | `UPDATE … SET lease_until=$ WHERE tenant_id=$ AND operation_key=$ AND owner_id=$ AND fencing_token=$ AND state='in_progress' AND lease_until > $now` | `affected==0` ⇒ `StaleOwner` |
| `complete` | `UPDATE … SET state='completed', completed_at=$now, response=$b64 WHERE …same five predicates…` | `affected==0` ⇒ `StaleOwner` |
| `abandon` | `DELETE FROM … WHERE …same five predicates…` | `affected==0` ⇒ `StaleOwner` |
| `purge_completed_before` | per row: `DELETE … WHERE tenant_id=$ AND operation_key=$ AND state='completed' AND completed_at < $cutoff` | sum of `affected`; a row re-created between select and delete is not matched |

### AD-5: A lost MVCC race maps to `Backend`, never to `StaleOwner`

**Choice**: On a mutation, `affected == 0` ⇒ `ReservationError::StaleOwner`. A raw error for which
`stoolap_common::is_write_conflict` returns `true` (`UniqueConstraint`, `TransactionAborted`,
`LockAcquisitionFailed`, `DatabaseLocked`, the pinned `"uncommitted changes from transaction"`
message) ⇒ `ReservationError::Backend("…; retry")`. Never silently proceed.

**Alternatives considered**: mapping write conflicts to `StaleOwner`.

**Rationale**: `StaleOwner` is a permanent verdict a caller acts on by giving up its lease. A
`DatabaseLocked` is transient and says nothing about ownership; reporting it as `StaleOwner`
would discard a lease the caller still legitimately holds. `affected == 0` is the only signal
that the statement genuinely ran and matched nothing — which is precisely
"not yours / not that token / no longer valid", the three cases the port collapses into
`StaleOwner`. The port has no transient variant, so `Backend` carries the retry hint in its
message, as Postgres already does (`reservation.rs:246-250`, `345-350`).

### AD-6: Expiry is read from an injected `Clock`, exactly as both existing implementations do

**Choice**: `StoolapOperationReservationStore::open(path, clock: Arc<dyn ego_domain::Clock>)`;
every expiry decision reads `clock.now()`, never a SQL `now()`. Adds `ego-domain` as an
**optional** dependency of `ego-persistence-stoolap` (layer-legal: `infrastructure → domain`; no
cycle — `ego-domain → ego-persistence-api` only).

**Rationale**: Not a preference — a requirement of the shared harness. `assert_reservation_store_conformance`
takes a factory returning `(S, Arc<TestClock>)` and positions the clock to drive expiry
deterministically. A store reading the database's clock cannot pass it. `Clock` lives in
`ego-domain`, not `ego-persistence-api` (`crates/domain/src/time/clock.rs:24`);
`ego-persistence-memory` already carries the same single-import edge for the same reason.

Same-process scope makes the skew caveat Postgres documents (`reservation.rs:3-34`) inapplicable
here: every owner reads one process clock.

### AD-7: A new `operation-reservation` Cargo feature, not a reuse of `event-sourcing`

**Choice**: `operation-reservation = ["dep:tokio", "dep:async-trait", "dep:chrono", "dep:ego-domain", "dep:base64"]`.
Module at `src/operation/reservation.rs` behind `#[cfg(feature = "operation-reservation")]`.

**Alternatives considered**: reusing `event-sourcing` (identical tokio/async-trait/chrono set).

**Rationale — from the actual Cargo.toml, not assumption**: the existing feature's own comment
(`Cargo.toml:11-15`) states its purpose: *"Optional so `Repository<A>`/`Snapshot` consumers of
this crate gain no new dependency."* This store needs two dependencies `event-sourcing` does not
provide — `ego-domain` (AD-6) and `base64` (AD-8). Adding them to `event-sourcing` would hand a
new crate edge to every event-sourcing consumer for a store it does not use, defeating the
feature's stated reason to exist. A reservation store is also not event sourcing; the name would
lie. `ego-domain` and `base64` are therefore declared `optional = true` and enabled only here.

### AD-8: Module placement `src/operation/reservation.rs`; `base64` for the response payload

**Choice**: `src/operation/mod.rs` + `src/operation/reservation.rs`, exported from the crate
root per `lib.rs:13-17`. `StoredServiceResponse` bytes are stored base64-encoded in a `TEXT`
column.

**Rationale**: Both the port (`persistence-api/src/operation/reservation.rs`) and the in-memory
adapter (`persistence-memory/src/operation/reservation.rs`) file this capability under
`operation/`; `persistence/` and `event_sourcing/` are the wrong families. For the payload,
Stoolap's `core::Value` has no binary variant — the exact dialect problem `StoolapEffectStore`
already solved with base64 TEXT (`effect-store/src/stoolap/mod.rs:12-14,51-52`); reusing that
choice avoids inventing a second encoding convention for one problem.

### AD-9: `sync=full` is verified with the strict parser, and the two S2 call sites move to it

**Choice**: Promote `dsn_declares_sync_full` (the query-string parser from
`effect-store/src/stoolap/mod.rs:187-191`) into `persistence/stoolap_common.rs`; use it in the
new store's `open()` and in `is_durable()`, and switch `snapshot.rs:74` and
`event_store.rs:213,263` from `db.dsn().contains("sync=full")` to it.

**Alternatives considered**: use the naive `contains` for consistency with S2; add the strict
parser for the new store only.

**Rationale**: The naive form matches a *path* that contains the text (`/data/no_sync=full/db`) —
the defect STOOLAP-EFFECT-01 already fixed once. Shipping the weak form knowingly is not an
option; shipping the strict form in only one of three stores leaves this crate with two
durability checks, which is the "second way to express durability" the proposal puts out of
scope. **Bounded and named so it is not read as scope creep**: two call sites, four lines,
no behavior change for any DSN `dsn_for` produces (it is the only producer).

### AD-10: `token_for_storage` / `token_from_storage` are re-implemented locally, not shared

**Choice**: Duplicate the two i64↔`FencingToken` guards (reject `raw <= 0`; `FencingExhausted`
rather than an unchecked cast) inside the new module.

**Rationale**: The originals are `pub(crate)` in `ego-persistence` and sharing them would force
an `ego-persistence-stoolap → ego-persistence` edge for twenty lines — the same trade
`StoolapEffectStore` already made for `dsn_for` (`effect-store/src/stoolap/mod.rs:161-171`).
Semantics MUST match exactly, including zero being rejected.

## Data Flow

```
caller ──reserve(req)──▶ StoolapOperationReservationStore
                             │  (owned params built here, before the boundary)
                             ▼
                       spawn_blocking ──▶ Database (cloned handle, one shared engine)
                             │                    │
                             │            operation_reservations table (sync=full WAL)
                             ▼
                     ReservationOutcome ◀── classify(affected, row, clock.now())
```

### Sequence: `reserve` — fresh, replay, conflict, and takeover

```
Owner-B          Store(async)      spawn_blocking       Stoolap engine        Clock
   │                  │                   │                    │               │
   │ reserve(req) ──▶ │                   │                    │               │
   │                  │ own params ──────▶│                    │               │
   │                  │                   │ INSERT … ON CONFLICT DO NOTHING    │
   │                  │                   │───────────────────▶│               │
   │                  │                   │◀── affected = 1 ───│               │
   │◀── Fresh(lease) ─┤                   │                    │               │
   │                  │                   │                    │               │
   │  ── otherwise: affected = 0 ──▶ the row already exists ──────────────────  │
   │                  │                   │ SELECT fingerprint, owner_id,      │
   │                  │                   │        fencing_token, lease_until, │
   │                  │                   │        state, response             │
   │                  │                   │───────────────────▶│               │
   │                  │                   │◀────── row ────────│               │
   │                  │                   │                                    │
   │                  │        fingerprint != req  ⇒ Conflict  (checked FIRST) │
   │                  │        state = 'completed' ⇒ Succeeded(decode(response))│
   │                  │                   │                    │               │
   │                  │                   │ now = clock.now()  │◀──────────────│
   │                  │                   │                    │               │
   │                  │   now <  lease_until ⇒ owner match ? OwnedInProgress   │
   │                  │                   │                   : OtherInProgress│
   │                  │                   │                    │               │
   │                  │   now >= lease_until ⇒ next = displaced.next()?        │
   │                  │                   │   (None ⇒ FencingExhausted)        │
   │                  │                   │ UPDATE … SET owner_id, fencing_token=next,
   │                  │                   │              lease_until            │
   │                  │                   │  WHERE state='in_progress'          │
   │                  │                   │    AND fencing_token = displaced    │  ← CAS
   │                  │                   │    AND lease_until  <= now          │  ← re-check
   │                  │                   │───────────────────▶│               │
   │                  │                   │◀── affected = 1 ───│               │
   │◀ TakenOver(lease, token=next) ───────┤                    │               │
   │                  │                   │                    │               │
   │       affected = 0 ⇒ a peer won the race in the window: re-SELECT,        │
   │       answer OwnedInProgress (this owner recovered) or OtherInProgress.   │
   │       Row vanished ⇒ Backend("…retry the reserve"), never an invented     │
   │       outcome. is_write_conflict(e) ⇒ Backend("…retry"), never StaleOwner.│
```

Owner-A, displaced, next calls `renew`/`complete`/`abandon` with its old token: the single
conditional statement matches zero rows (`fencing_token` no longer equals) ⇒ `StaleOwner`, and
the reservation is provably unmodified because nothing but that statement could have modified it.

## File Changes

| File | Action | Description |
|---|---|---|
| `crates/persistence-api/src/operation/reservation.rs` | Modify | (a) `is_durable()` default `false` + doc; `Arc<T>` forwarding impl for all 7 methods; tests: bare impl defaults false, `Arc<Concrete>` as a generic `S` forwards true (AD-2) |
| `crates/persistence-memory/src/operation/reservation.rs` | Modify | (a) explicit `fn is_durable(&self) -> bool { false }` with a doc line saying volatile is the honest answer; test |
| `crates/persistence/src/postgres/reservation.rs` | Modify | (a) override `is_durable() -> true`; test |
| `crates/testkit/src/reservation.rs` | Review only | Re-exports the in-memory store; no double to change. Confirm no other test double implements the port |
| `crates/persistence-stoolap/Cargo.toml` | Modify | (b) `ego-domain`/`base64` optional deps; `operation-reservation` feature; `[[test]]` `required-features` for the new integration test |
| `crates/persistence-stoolap/src/lib.rs` | Modify | (b) `#[cfg(feature = "operation-reservation")] pub mod operation;` + crate-root `pub use` |
| `crates/persistence-stoolap/src/operation/mod.rs` | Create | (b) `pub mod reservation;` |
| `crates/persistence-stoolap/src/operation/reservation.rs` | Create | (b) `StoolapOperationReservationStore` + colocated unit tests |
| `crates/persistence-stoolap/src/persistence/stoolap_common.rs` | Modify | (AD-9) add `dsn_declares_sync_full` + its unit test |
| `crates/persistence-stoolap/src/persistence/snapshot.rs`, `src/event_sourcing/event_store.rs` | Modify | (AD-9) 3 call sites switch to the strict parser |
| `crates/persistence-stoolap/tests/reservation_conformance.rs` | Create | (b) shared harness + reopen-durability test |
| `crates/service-sdk/src/runtime/builder.rs` | Modify | (c) `validate_operation_reservation_profile()`; one line in `validate_persistence_profile()`; gate matrix tests |

## Interfaces / Contracts

```rust
// (a) crates/persistence-api/src/operation/reservation.rs
#[async_trait]
pub trait OperationReservationStore: Send + Sync {
    /// Whether reservations written through this store survive a process restart.
    ///
    /// Defaults to `false`: honest for every implementation that has not considered
    /// the question. `Profile::Production` reads this; a durable implementation
    /// overrides it to `true`.
    fn is_durable(&self) -> bool { false }
    // ... existing 7 methods unchanged ...
}

#[async_trait]
impl<T: OperationReservationStore + Send + Sync + ?Sized> OperationReservationStore
    for std::sync::Arc<T>
{
    /// **Load-bearing in a generic context** (AD-2): omitting this makes
    /// `Arc<ConcreteStore>` used as an `S: OperationReservationStore` report the
    /// trait's `false` default regardless of what it wraps.
    fn is_durable(&self) -> bool { (**self).is_durable() }
    // forwards reserve/renew/complete/abandon/purge_completed_before/
    // oldest_completed/probe — oldest_completed MUST be forwarded, per the port's
    // own "a wrapper MUST forward this rather than inherit the default".
}
```

```rust
// (c) crates/service-sdk/src/runtime/builder.rs
fn validate_persistence_profile(&self) -> Result<(), RuntimeError> {
    self.validate_effect_store_profile()?;
    self.validate_read_side_progress_profile()?;
    self.validate_read_side_claim_profile()?;
    self.validate_operation_reservation_profile()?;   // new, appended
    Ok(())
}

/// Under `Profile::Production`, a **registered** reservation store must be durable
/// (AD-3). Registration is itself the composition-visible signal that this runtime
/// dispatches through the store — `build()` assembles `ReservationConfig` from it
/// regardless of enforcement mode. Absence is deliberately not checked here:
/// `validate_idempotency` already owns "MandatoryKey requires a store", and
/// restating it would be the second parallel definition PROD-014A avoids.
fn validate_operation_reservation_profile(&self) -> Result<(), RuntimeError> {
    let Some(store) = self.idempotency_reservation_store.as_ref() else {
        return Ok(());
    };
    persistent_entity::profile::require_durably_configured(
        self.profile,
        store.is_durable(),
        "durable operation reservation store (OperationReservationStore)",
        "AppBuilder::operation_reservation_store(store) (or \
         RuntimeBuilder::with_operation_reservation_store(..)), passing a store whose \
         is_durable() returns true",
    )?;
    Ok(())
}
```

```sql
-- (b) created by open(); UNIQUE, never PRIMARY KEY: Stoolap parses a table-level
-- composite PRIMARY KEY but does NOT enforce it, while UNIQUE is enforced and is
-- what ON CONFLICT matches (effect-store/src/stoolap/mod.rs:228-236).
CREATE TABLE IF NOT EXISTS operation_reservations (
    tenant_id     TEXT      NOT NULL,   -- '' = systemwide (stoolap_common::encode_tenant)
    operation_key TEXT      NOT NULL,
    fingerprint   TEXT      NOT NULL,
    owner_id      TEXT      NOT NULL,
    fencing_token INTEGER   NOT NULL,   -- i64; token_for_storage/token_from_storage guard it
    lease_until   TIMESTAMP NOT NULL,
    state         TEXT      NOT NULL,   -- 'in_progress' | 'completed'
    completed_at  TIMESTAMP,
    response      TEXT,                 -- base64; NULL while in_progress
    UNIQUE (tenant_id, operation_key)
)
```

```rust
// (b) constructor and durability
impl StoolapOperationReservationStore {
    pub async fn open(path: &Path, clock: Arc<dyn Clock>) -> Result<Self, ReservationError>;
    async fn run_blocking<F, R>(&self, f: F) -> Result<R, ReservationError> where /* spawn_blocking */;
}
impl OperationReservationStore for StoolapOperationReservationStore {
    fn is_durable(&self) -> bool { dsn_declares_sync_full(self.db.dsn()) }
    // probe(): `SELECT 1 FROM operation_reservations LIMIT 1`, row discarded — read-only,
    //          and proves the schema exists, not merely that the engine answers.
    // oldest_completed(): `SELECT MIN(completed_at) … WHERE state='completed'`;
    //          NULL ⇒ Empty (a real answer), never Unsupported.
}
```

**Tenant sentinel**: `stoolap_common::encode_tenant` maps `None` to `""`. Chosen over
Postgres's `tenant_id IS NOT DISTINCT FROM $1` because this crate already established the
sentinel to avoid SQL NULL comparison semantics, and `TenantId::new("")` is rejected, so no real
tenant can collide.

**Purge dialect constraint**: `DELETE … WHERE col IN (SELECT … LIMIT n)` silently deletes **zero**
rows on Stoolap 0.4.0 (`effect-store/src/stoolap/mod.rs:292-301`). `purge_completed_before` must
therefore `SELECT tenant_id, operation_key … WHERE state='completed' AND completed_at < $cutoff
LIMIT $batch`, then delete each by its own equality predicate **re-asserting eligibility**
(AD-4 table). Re-asserting is a deliberate strengthening over `run_retention`'s delete-by-id: a
reservation key is re-creatable after `abandon`, so a bare key match could delete a *new*
reservation. No `ORDER BY`: selection within a batch is outside the contract.

## Concurrency Scope — what is claimed and what is not

| Scenario | Supported | Basis |
|---|---|---|
| Same process, many async tasks, one store instance | **Yes** | Every mutation is one conditional statement under Stoolap MVCC (AD-4); `spawn_blocking` over cloned `Database` handles sharing one engine — the same basis for `StoolapEffectStore`'s `concurrent_local_safe: true` |
| Same process, two runtime instances / two store instances at the same path | **Yes** | Stoolap's process-global registry shares one live engine per DSN while a handle is alive (`effect-store/src/stoolap/mod.rs:200-215`); all owners read one process clock, so no expiry skew. Must be *tested*, not assumed (TS-4) |
| Multiple OS processes over the same file | **Not supported, not tested** | Two engines over one file; nothing in-tree establishes cross-process locking. No claim is made |
| Multi-node | **Not supported** | Same honesty precedent as `StoolapEffectStore`'s `multi_node_safe: false` (`effect-store/tests/conformance.rs:296-302`) |

Fencing bounds what it can: it makes the *reservation outcome* authoritative. It does not cancel
an in-flight external effect a displaced owner already issued — the same limit Postgres states
(`reservation.rs:22-31`). Say this in the module doc; do not soften it.

## Testing Strategy

| Layer | What to test | Approach |
|---|---|---|
| Unit — port (a) | Bare impl inherits `false`; `Arc<Concrete>` as a generic `S` reports `true`; `Arc` forwards all 7 methods incl. `oldest_completed` | Colocated `#[cfg(test)]`, mirroring `dedup.rs:116-175`. The `Arc` durability test MUST use a generic `S`, not `Arc<dyn _>` (AD-2) |
| Unit — implementors (a) | in-memory reports `false`; Postgres reports `true` | Colocated; Postgres's needs no pool (`is_durable` is pure) |
| Unit — Stoolap (b) | `open()` refuses a non-`sync=full` engine; `is_durable()` true; `dsn_declares_sync_full` rejects a path containing the text | Colocated, `tempfile` per test + `stoolap::test_failpoints::FailpointGuard`, as `snapshot.rs:179-197` does |
| Integration (b) | `assert_reservation_store_conformance` against a Stoolap store + `TestClock` | `tests/reservation_conformance.rs`, `required-features = ["operation-reservation"]`. Same harness the in-memory store passes — no second copy of the contract |
| Integration (b) | Reopen durability: reserve → drop store → reopen same path → owner, fencing token, lease bound, tenant scope intact; a stale fence still rejected after reopen | New test; the factory shape `StoolapDurableStoreFactory` uses |
| Integration (b) | TS-4 same-process, two store instances at one path: A reserves, clock advances past the lease, B (separate instance) takes over with a strictly greater token, A's fence then fails `StaleOwner` | Proves the row in the table above rather than asserting it |
| Integration (b) | Tenant isolation: identical key under tenant A / tenant B / systemwide are three reservations | Explicit — the sentinel encoding is where this could silently break |
| Integration (b) | Purge: batch bound honoured, in-progress never removed, count is rows actually deleted, drainage over successive calls | Covered by the harness's purge group; add one Stoolap-specific test proving the two-step batch actually deletes (guards the `IN (SELECT … LIMIT)` dialect trap) |
| Unit — gate (c) | Matrix `{Dev, Production} × {no store, volatile store, durable store}` — only `Production` + volatile refuses | `builder.rs` tests, cloning `validate_read_side_claim_profile_matrix`'s shape with `compat()` and a `StubReservationStore(bool)` |
| Unit — gate (c) | `Production` + volatile store under **`Compatibility`** mode still refuses | The test that distinguishes AD-3 from the rejected `MandatoryKey`-triggered alternative. Without it the decision is unpinned |
| Unit — gate (c) | Rejection names the capability and `with_operation_reservation_store` | Mirrors `validate_read_side_claim_profile_rejects_volatile_claim_store` |
| Unit — gate (c) | `build()` panics and `try_build()` returns the same refusal | Both paths, as every sibling gate does |

TDD is on (`openspec/config.yaml`): each row lands RED first. `cargo test --workspace` does not
enable `operation-reservation`; the slice-(b) tests must also be run with
`--features operation-reservation`, and CI's `--all-targets` must still compile without it —
which is why the new integration test carries `required-features` (`Cargo.toml:50-52` precedent).

## Threat Matrix

N/A — no routing, shell, subprocess, VCS/PR automation, executable-file classification, or
process-integration boundary. The one adjacent concern, caller-controlled text reaching SQL, is
handled by the same rule Postgres states: every value is bound as `$N`, never interpolated —
an `OperationKey` is client-supplied.

## Migration / Rollout

No data migration. `open()` issues `CREATE TABLE IF NOT EXISTS`, so a new database and an
existing S1/S2 database at the same path both work. Slice (a) is additive with a `false` default,
so no external implementor breaks. Slice (b) is purely additive and feature-gated off by default.
Slice (c) is the only behavior change and reverts alone; a host that was registering a volatile
reservation store under `Profile::Production` will fail at build time with a message naming the
fix — which is the intent.

Review-budget slicing: (a) ≈ 150 lines, (b) ≈ 550 lines, (c) ≈ 200 lines. Three stacked PRs;
(b) is the one at risk of exceeding 400 and may split into (b1) store + conformance and
(b2) reopen/concurrency/tenant tests.

## Open Questions

- [ ] Does Stoolap 0.4.0 support `MIN(completed_at)`? `MAX`+`COALESCE` is proven in-tree
      (`event_store.rs:64-65`), `MIN` is not. If unsupported, `oldest_completed` falls back to
      `SELECT completed_at … WHERE state='completed' ORDER BY completed_at ASC LIMIT 1` — same
      answer, no contract change. Resolve by experiment in slice (b), not by assumption.
- [ ] Does a nullable `TIMESTAMP` column read back as `Option<DateTime<Utc>>` through Stoolap's
      row API? `effect-store`'s schema has nullable timestamps, so the shape is established;
      confirm the exact accessor before writing `completed_at` handling.
- [ ] Non-blocking follow-up, explicitly out of this change: nothing renews a reservation lease
      automatically (port doc, `reservation.rs:19-27`). A Stoolap-backed deployment inherits that
      unchanged; it is not a gap this change introduces.
