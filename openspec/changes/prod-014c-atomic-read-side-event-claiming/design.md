# Design: PROD-014C — Atomic Read-Side Event Claiming

> Canonical / source of truth. Spanish review companion: `design.es.md` (1:1 identifiers).
>
> **Inputs**: `proposal.md` (D-1 … D-12, IS-1 … IS-8, OOS-1 … OOS-6, R-1 … R-5, SC-1 … SC-7,
> Required Semantics) and `exploration.md`. D-1 … D-12 are locked and are not relitigated
> here. This document decides **how**: mechanism, port shape, SQL, session/scheduler wiring,
> gate shape, schema, and test shape.
>
> **Baseline read**: `develop` @ `30e42ab`. Every file:line below was read on this baseline,
> not recalled.
>
> **Wording discipline (R-1, SC-6)**: this document says *single valid processing ownership*,
> *atomic claiming*, and *execution exclusion*. It never says exactly-once, concurrency-safe,
> or multi-replica-safe as an achieved property.

## Technical Approach

One new port (`ReadSideClaimStore`) in `crates/persistence-api/src/read_side/claim.rs`, one
durable adapter over one new table (`projection_claims`, migration `016`), and one
fence-verified claim wrapped around the existing `ReadSideSession::execute()` body. Claim
identity is `(projection_id, tag, tenant)` — byte-for-byte `projection_offsets`' primary key
(D-1).

Acquisition is **one** statement: `INSERT … ON CONFLICT (…) DO UPDATE … WHERE lease_until <= now
RETURNING fencing_token`. Insert if absent, take over only if the incumbent lease has lapsed,
mint a strictly greater token in the same statement, and affect zero rows otherwise. There is no
check-then-act window of this design's making, and no Rust logic between two queries.

The claim is taken **per stream per batch**, not per event (R-3): `try_claim` before `fetch`,
`renew` immediately before the commit phase, `release` on every exit path. A refused worker
returns `Ok(None)` and calls neither `fetch` nor the handler.

`OffsetStore` and `DedupStore` are consumed unchanged (D-2, D-11). The scheduler's public
`TagScheduler` trait signature is unchanged.

---

## The Mechanism Comparison (A–E)

The five candidates are checked against the seven required properties. The decision criterion
is the proposal's: **the minimal mechanism that rigorously satisfies all of them.**

### The property that decides it: stale-worker safety

Everything else is satisfiable by more than one candidate. This one is not, and the reason is
structural rather than a matter of degree.

`write_offset` is a plain upsert with last-write-wins by contract
(`crates/persistence/src/postgres/read_side_offset.rs:5-14`, PROD-014B AD-3). A stale worker
that resumes and upserts its own smaller `Offset::sequence(v_A)` over a live owner's `v_B > v_A`
does not merely waste a fetch. The re-fetched range `v_A+1 … v_B` is already marked in
`projection_dedup`, so `unique_events` is empty and `execute()` returns at
`session.rs:130-132` — **before** `write_offset` is reached. The offset therefore stays rewound
and re-advances only once new events appear past the batch window; with more than `batch_size`
events in the gap the projection stalls indefinitely. This is exactly the failure the
`session.rs:81-87` doc comment describes for a non-resuming offset, reached from the other side.

So a mechanism must let a worker **ask the database whether it is still the owner and be told
no**. That question is only answerable if the worker holds something the database can compare
against the current row.

| | (A) `FOR UPDATE` | (B) advisory lock | (C) claim table + fence | (E) `FOR UPDATE SKIP LOCKED` |
|---|---|---|---|---|
| Mutual exclusion | Yes, while the tx is open | Yes, while the session holds it | Yes, by the PK + the `WHERE` | Yes, same as (A) |
| Atomic acquisition (one primitive) | Needs `INSERT … ON CONFLICT` first to materialise a row to lock, then `FOR UPDATE` — **two** statements | One `pg_advisory_lock` call | **One** `INSERT … ON CONFLICT DO UPDATE … WHERE … RETURNING` | Same two statements as (A) |
| Crash recovery | Excellent — dies with the tx, no lease, no clock | Auto-release on connection close, but "connection closed" ≠ "worker stopped"; reaping a half-open backend is a TCP/keepalive setting, not a configured bound | Lease expiry against an injected `Clock` (D-4) | Same as (A) |
| **Stale-worker safety (fencing)** | **No — see below** | **No — see below** | **Yes** — the worker holds `(owner_id, fencing_token)` and every mutation re-verifies the full triple in its own `WHERE` | **No** — inherits (A) |
| Multi-node safety | Yes (no in-memory state) | Yes | Yes | Yes |
| Ordering preservation | Yes (claim is per stream) | Yes | Yes | Yes |
| Retry under contention | Blocks head-of-line, or `NOWAIT` to refuse | Blocks, or `try_advisory_lock` to refuse | Refuses immediately: `rows_affected() == 0`, no waiting, no lock held | Skips — but see below |
| Connection cost | Pins one pooled connection per active stream for the batch's whole duration, **including `handler.handle()`** (arbitrary user I/O) | Same, plus a dedicated pinned connection | One round trip per statement; nothing held between | Same as (A) |
| Observability | `pg_locks` shows a tx lock, not which worker owns which stream | `pg_locks` shows `classid`/`objid` — the **hash**, not the identity | `SELECT * FROM projection_claims` names owner, token, and lease | Same as (A) |

#### Why (A) provably cannot handle stale-worker safety

A lock is not a token. When the lock is released — by crash, by
`idle_in_transaction_session_timeout`, by a pooled-connection recycle, or by a partition where
the backend is reaped — PostgreSQL frees it while the holder's Rust future is still parked inside
`handler.handle()`. Nothing tells the future. When it resumes and asks "am I still the owner?",
the only thing it can do is open a new transaction and re-lock — **which succeeds**, because the
previous lock is gone. The mechanism answers *yes* to a worker that has already been replaced,
and the worker proceeds to rewind the offset.

The only way to close it under (A) is to run `mark_seen` and `write_offset` **inside the same
transaction that holds the lock**, which requires threading a transaction handle through
`OffsetStore`/`DedupStore`. `crates/domain` has zero PostgreSQL awareness by hexagonal design
and no port carries a transaction handle (`exploration.md` §"Transactional guarantees — none, and
why"); adding one reopens the two archived PROD-014B contracts (**D-2**) and is precisely the
cross-table atomicity **D-11** excludes. So (A) is not "weaker here" — it is unimplementable
within this change's locked decisions.

#### Why (B) provably cannot either, plus three failures of its own

Same root cause: no token exists anywhere, so "am I still the owner?" is again a re-acquire that
succeeds. Adding a token means adding a table — at which point the table is the mechanism and the
advisory lock is decoration. On top of that:

1. **Pool incompatibility.** `pg_advisory_lock` is *session*-scoped, bound to a backend
   connection. `sqlx::PgPool` chooses the connection per query, so the lock is acquired on C1
   while the next statement runs on C2. Correctness requires holding a dedicated
   `pool.acquire()` for the claim's whole life — (A)'s connection-pinning cost, with a lock that
   no longer dies with a transaction. `pg_advisory_xact_lock` is transaction-scoped and
   therefore collapses into (A) exactly, inheriting all of it.
2. **Silent hash collisions.** Keys are `bigint`; `(projection_id, tag, tenant)` is three
   strings, so it must be hashed to 64 bits. A collision mutually excludes two unrelated
   streams — a stall that looks like an idle projection.
3. **Undiagnosable.** `pg_locks` exposes the hash, not the identity, so an operator cannot
   answer "which worker owns `users-by-tenant:tenant-a`" from any system view. Collision (2) is
   therefore invisible from the outside.

#### Why (E) is a red herring here

`reservation.rs:485-501` uses `FOR UPDATE SKIP LOCKED` in `purge_completed_before`, and its own
comment states the scope precisely: it is a **progress** guarantee, not a safety one — "without
it two workers still cannot remove the same row twice … What it provides is that a worker whose
batch could be filled from unlocked rows fills it", guarded by `purge_progress_postgres.rs`.
That is competing consumers draining interchangeable rows.

Read-side streams are not interchangeable and there is no queue. `tag_provider()`
(`scheduler.rs:308`) decides which `(tag, tenant)` pairs *this process* serves, and
`ReadSideStore::fetch` guarantees ordering *within* one stream. A worker must poll its own
stream, not whichever row happens to be unlocked. And as a modifier on `FOR UPDATE`, (E)
inherits (A)'s transaction-lifetime and no-token problems verbatim.

The one useful property of (E) — **refused, not blocked** — is delivered by (C) natively:
`rows_affected() == 0` refuses without taking or waiting on any lock.

#### (D) Lease + fencing: how, not whether

D-3 already concluded fencing is required. Formalised:

- **Token shape** — reuse `FencingToken`
  (`crates/persistence-api/src/operation/reservation.rs:225-267`): `u64`, starts at 1,
  `next()` is `checked_add` and reports exhaustion rather than wrapping. Its whole documented
  promise — "a wrapped token could compare equal to a fence a prior owner still holds" — is the
  same promise needed here (AD-3).
- **Where it is minted** — inside the same `ON CONFLICT DO UPDATE` that re-owns the row, as
  `projection_claims.fencing_token + 1`. Never read-then-increment in Rust.
- **Where it is checked** — in the `WHERE` clause of every mutating statement (`renew`,
  `release`), alongside `owner_id` and the full identity. Verification and mutation are one
  statement, exactly `reservation.rs`'s `mutate_owned` shape (`:578-605`).
- **On mismatch** — zero rows affected ⇒ `ClaimError::StaleOwner`, and the row is guaranteed
  unmodified. The session treats `StaleOwner` from the pre-commit `renew` as an abort: it writes
  no dedup marker and no offset (AD-6).

### Decision

**(C) — an atomic durable claim table with a lease and a fencing token.** It is the only
candidate that satisfies stale-worker safety at all, and the only one whose acquisition is a
single statement. It is also the only one that does not pin a pooled connection across arbitrary
user code, and the only one an operator can query.

It is minimal in the sense the criterion asks for: **three methods, not four.**
`reservation.rs`'s `complete` exists because a reservation stores a response for later replay.
A read-side claim stores nothing — the offset and the dedup markers are the other two ports'
property — so `complete` collapses into `release`. The proposal's illustrative
`try_claim`/`renew`/`complete`/`release` is cut to `try_claim`/`renew`/`release` (plus
`is_durable`, which the gate reads).

---

## Component Map

```
crates/persistence-api/src/read_side/
├── claim.rs                          NEW  ReadSideClaimStore + types + Arc<T> impl  (AD-1..AD-4)
└── mod.rs                            MOD  `pub mod claim;`

crates/domain/src/read_side/
├── mod.rs                            MOD  re-export `claim` at its original path shape
└── session.rs                        MOD  ReadSideClaiming knob + claim/renew/release (AD-6)

crates/persistence/src/postgres/
├── migrations/016_create_projection_claims.sql   NEW   (AD-8)
├── migrations.rs                     MOD  one const + one registry entry
├── read_side_claim.rs                NEW  PostgreSQLReadSideClaimStore              (AD-5)
├── reservation.rs                    MOD  `token_from_storage` → pub(crate)         (AD-3)
└── mod.rs                            MOD  one `pub use`

crates/runtime/src/read_side/scheduler.rs         MOD  ProjectionSpec::claims knob    (AD-7)
crates/service-sdk/src/runtime/builder.rs         MOD  slot + validate_read_side_claim_profile (AD-9)
crates/service-sdk/src/app/mod.rs                 MOD  AppBuilder::read_side_claims   (AD-9)
examples/reference-app/src/read_side/mod.rs       MOD  retire the PROD-014C promise    (IS-8)
integration-tests/tests/infrastructure/
└── read_side_claiming_postgres.rs    NEW  the contention suite                      (AD-10)

crates/persistence-api/src/read_side/{offset,dedup,store}.rs   UNTOUCHED  (D-2)
crates/persistence/src/postgres/{read_side_offset,read_side_dedup}.rs  UNTOUCHED  (D-2)
```

## Data Flow

```
ReadSideSession::execute()                              PostgreSQL
──────────────────────────                              ──────────
 0  try_claim(id, owner, now+lease) ───────────────▶  INSERT INTO projection_claims …
        │                                              ON CONFLICT (pid,tag,tenant) DO UPDATE
        │                                                SET owner_id=EXCLUDED.owner_id,
        │                                                    fencing_token=…+1, lease_until=…
        │                                              WHERE projection_claims.lease_until <= $now
        │                                              RETURNING fencing_token
        ├── None (0 rows) ──▶ Ok(None). No fetch. No handler. ◀── refused
        ▼ Some(fence)
 1  read_offset  ─┐
 2  fetch         ├─ unchanged; ascending event_version per (tenant, tag) preserved
 3  dedup filter ─┘   because only the fence holder ever reaches them
        ▼
 4  handler.handle(unique_events)          ← the window PROD-014B AD-6 named, now inside a claim
        ▼
 5  renew(fence, now+lease) ───────────────────────▶  UPDATE … SET lease_until=$1
        │                                             WHERE pid,tag,tenant AND owner_id=$
        │                                               AND fencing_token=$ AND lease_until > $now
        ├── StaleOwner (0 rows) ──▶ abort. No mark_seen. No write_offset. ◀── fenced out
        ▼
 6  mark_seen ×n ; write_offset ; on_batch_completed   (unchanged ports, D-2)
        ▼
 7  release(fence) ────────────────────────────────▶  UPDATE … SET lease_until = $now
                                                       (same full-fence WHERE)
                                                       → immediately claimable again
```

Steps 1-6 also run under `release` on every exit path, including the error path (AD-6).

---

## Architecture Decisions

### AD-1 — The port: `ReadSideClaimStore`, four methods, in `persistence-api`

**Decision** — `crates/persistence-api/src/read_side/claim.rs`:

```rust
/// The capability port through which one worker obtains single valid
/// processing ownership of a (projection_id, tag, tenant) stream.
///
/// Every mutating call (`renew`, `release`) MUST verify the full
/// `claim_id + owner_id + fencing_token` triple inside the same statement
/// that mutates. A caller whose claim was taken over receives
/// `ClaimError::StaleOwner` and its call MUST leave the claim unmodified.
#[async_trait::async_trait]
pub trait ReadSideClaimStore: Send + Sync {
    /// Whether claims obtained through this store survive a process restart.
    /// Defaults to `false`, mirroring `OffsetStore::is_durable`
    /// (`offset.rs:62-64`). `Profile::Production` reads this (IS-6).
    fn is_durable(&self) -> bool {
        false
    }

    /// Obtains the claim, or reports that a live claim already holds it.
    ///
    /// `Ok(None)` is a refusal, not a failure: another worker holds an
    /// unexpired lease. The caller MUST NOT fetch or invoke the handler.
    /// `Ok(Some(fence))` is granted, whether fresh or taken over from a
    /// lapsed owner; the fence carries a strictly greater token than any
    /// this identity previously issued.
    ///
    /// `lease_until` is computed by the caller (`clock.now() + configured
    /// lease`), never by the store — `ReserveRequest::lease_until`'s rule
    /// (`operation/reservation.rs:332-335`) and D-4.
    async fn try_claim(
        &self,
        claim_id: &ClaimId,
        owner_id: &OwnerId,
        lease_until: DateTime<Utc>,
    ) -> Result<Option<ClaimFence>, ClaimError>;

    /// Extends an owned, still-valid claim to `lease_until`.
    ///
    /// MUST reject a stale fence AND an already-lapsed lease with
    /// `StaleOwner`, leaving the claim unmodified — a lapsed holder
    /// resurrecting its claim would defeat a takeover that was already
    /// legitimate (`operation/reservation.rs:74-81`).
    async fn renew(
        &self,
        fence: &ClaimFence,
        lease_until: DateTime<Utc>,
    ) -> Result<(), ClaimError>;

    /// Releases an owned, still-valid claim, making the stream immediately
    /// claimable without waiting for expiry. Same fence rule as `renew`.
    async fn release(&self, fence: &ClaimFence) -> Result<(), ClaimError>;
}
```

**Criteria**: (a) `#[async_trait]` and a `fn is_durable() -> bool { false }` default are the
exact `OffsetStore` conventions (`offset.rs:54-64`) — a defaulted `false` is honest for every
implementation that has not considered the question, and the gate reads it; (b) `try_claim`
takes no fence because there is nothing to prove yet, and `renew`/`release` take nothing *but*
the fence because the fence carries the identity; (c) three verbs, per the Decision above.

**Rejected — a `ClaimOutcome` enum.** `ReservationOutcome` needs six variants because a
reservation carries a fingerprint and a stored response, so it must distinguish `Conflict`,
`Succeeded`, `OwnedInProgress`, and `OtherInProgress`. A claim is binary: granted or refused.
`Option<ClaimFence>` says exactly that with no invented names. `Fresh` vs `TakenOver` is
likewise not distinguished — no requirement in this change reads it, and takeover is directly
observable in the row as `fencing_token > 1` (SC-2 asserts it there).

**Rejected — a method on `OffsetStore`.** D-2, locked.

### AD-2 — Types: `ClaimId` and `ClaimFence` are new; `OwnerId` and `FencingToken` are reused

**Decision**:

```rust
/// The claim identity — exactly `projection_offsets`' primary key (D-1).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ClaimId {
    pub projection_id: String,
    pub tag: EventTag,
    pub tenant: String,
}

/// The full verification triple every mutating call presents.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaimFence {
    pub claim_id: ClaimId,
    pub owner_id: OwnerId,
    pub fencing_token: FencingToken,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ClaimError {
    /// The presented fence no longer matches the current claim — typically
    /// because the claim was taken over. The claim is unmodified.
    #[error("stale owner: the presented fence no longer matches the current claim")]
    StaleOwner,
    /// No strictly greater token can be minted, so takeover cannot proceed
    /// safely. Unreachable in practice; represented rather than wrapped.
    #[error("fencing token sequence exhausted for this claim")]
    FencingExhausted,
    #[error("transient claim store error: {0}")]
    Transient(String),
    #[error("fatal claim store error: {0}")]
    Fatal(String),
}
```

**Criteria**: `OwnerId` (`operation/reservation.rs:203-217`) documents itself as "identifies the
caller instance holding (or attempting to hold) a lease" — nothing operation-specific. Same for
`FencingToken`, whose checked-`next()`/exhaustion semantics and unit tests
(`:489-521`) are precisely what this change needs and would otherwise be re-derived. Both live in
`ego-persistence-api`, the same crate as the new port, so reuse crosses no boundary.

**Rejected — reusing `ReservationError`.** Its `Backend(String)` collapses the
`Transient`/`Fatal` split that is the read-side convention across `OffsetStoreError`
(`offset.rs:42-49`) and `DedupStoreError`, and that PROD-014B AD-8's `is_fatal` predicate exists
to compute. The helper is reused (AD-5); the type is not.

**Rejected — duplicating `OwnerId`/`FencingToken` under read-side names.** Two identical
monotonic-token types with two copies of the exhaustion argument is strictly more surface saying
strictly less.

### AD-3 — `Arc<T>` blanket forwarding impl, with `is_durable()` forwarded explicitly

**Decision**: the same blanket impl `OffsetStore` carries (`offset.rs:86-119`):

```rust
#[async_trait::async_trait]
impl<T: ReadSideClaimStore + Send + Sync + ?Sized> ReadSideClaimStore for std::sync::Arc<T> {
    /// **Load-bearing.** Omitting this silently inherits the trait's `false`
    /// default, so every registered store would be classified volatile no
    /// matter what the host wrapped — the gate would refuse a correct durable
    /// composition and pass nothing (PROD-014A EC-2).
    fn is_durable(&self) -> bool {
        (**self).is_durable()
    }
    // … three forwarding method bodies
}
```

**Criteria**: D-2 cites this requirement directly. The composition root holds the store as
`Arc<dyn ReadSideClaimStore + Send + Sync>` and hands that same value to `ProjectionSpec`, so the
registered value and the spawned value must be the same value. Pinned by the same
`arc_forwards_is_durable` landmine test `offset.rs:189-197` carries.

Also in this AD: `crates/persistence/src/postgres/reservation.rs`'s private `token_from_storage`
(`:124-138`) becomes `pub(crate)`. It is six lines whose whole content is the argument that a
stored token must be positive and that `u64::try_from` accepts zero — re-deriving it in a second
adapter is how the two copies diverge.

### AD-4 — `try_claim` refusal is `Ok(None)`, and the session turns it into `Ok(None)` too

**Decision**: a refused claim is not a `ProjectionError`. `ReadSideSession::execute()` already
returns `Ok(None)` for "nothing advanced this tick" (`session.rs:112-114`, `:130-132`), and a
refusal is exactly that.

**Criteria**: on a two-replica deployment the non-owning replica is refused on **every** tick for
**every** stream it does not own. Classifying that as an error would drive `on_error`
(`scheduler.rs:321`) at the poll frequency, on the majority of replicas, permanently — turning
the normal steady state into a log flood. The claim row is the observability surface instead: it
names the owner, the token, and the lease (AD-8).

**Rejected — a new `ProgressReporter` method or a new `ProjectionError` variant.** Both expand a
shipped port to report a non-event, and neither is required by any success criterion.

### AD-5 — The adapter: one statement per operation, `is_fatal` reused

**Decision** — `crates/persistence/src/postgres/read_side_claim.rs`,
`PostgreSQLReadSideClaimStore { pool: PgPool, clock: Arc<dyn Clock> }`, manual `Debug` printing
only the pool (`reservation.rs:75-81`), `is_durable() -> true`.

`try_claim` — **one** statement, the whole mechanism:

```rust
let token: Option<i64> = sqlx::query_scalar(
    r#"INSERT INTO projection_claims
           (projection_id, tag, tenant, owner_id, fencing_token, lease_until, claimed_at)
       VALUES ($1, $2, $3, $4, 1, $5, NOW())
       ON CONFLICT (projection_id, tag, tenant) DO UPDATE
          SET owner_id      = EXCLUDED.owner_id,
              fencing_token = projection_claims.fencing_token + 1,
              lease_until   = EXCLUDED.lease_until,
              claimed_at    = NOW()
        WHERE projection_claims.lease_until <= $6
       RETURNING fencing_token"#,
)
.bind(&claim_id.projection_id).bind(claim_id.tag.value()).bind(&claim_id.tenant)
.bind(owner_id.as_str()).bind(lease_until).bind(self.clock.now())
.fetch_optional(&self.pool).await.map_err(claim_error)?;
```

Behaviour, case by case:

| Row state | Outcome |
|---|---|
| Absent | INSERT path, token `1`, `Some(fence)` — fresh |
| Present, `lease_until <= now` | DO UPDATE fires: new owner, token strictly `+1`, `Some(fence)` — takeover |
| Present, `lease_until > now` | the DO UPDATE `WHERE` suppresses; zero rows; `RETURNING` yields nothing → `None` — refused |
| Two concurrent inserts | one wins the unique index; the other's `ON CONFLICT` path waits on that row lock and, under READ COMMITTED, re-evaluates its `WHERE` against the **committed winner row**, sees a live lease, and is refused |

That last row is the same reasoning `reservation.rs:283-294` states for its takeover predicate —
"a caller that waited on the row lock is judged against the row that exists, not the row it
remembers" — except here it happens *inside one statement* instead of across a read and a write,
so this design has no window of its own to defend at all.

`renew` and `release` share one `mutate_owned`-shaped private helper, structurally identical to
`reservation.rs:578-605`:

```sql
-- renew                                        -- release
UPDATE projection_claims                        UPDATE projection_claims
   SET lease_until = $1                            SET lease_until = $1   -- bound to now
 WHERE projection_id = $2 AND tag = $3           WHERE …identical fence WHERE…
   AND tenant = $4 AND owner_id = $5
   AND fencing_token = $6
   AND lease_until > $7
```

Zero rows affected ⇒ `ClaimError::StaleOwner`, from all three of "not yours", "not that token",
and "no longer valid", which the port deliberately does not distinguish
(`reservation.rs:576-577`).

**Release is an expiry, never a `DELETE`.** A `DELETE` would make the next `try_claim` take the
INSERT path and **reset the token to 1**, breaking strict monotonicity across the release
boundary — a stale fence from two generations back could then compare equal to a live one, and
the only thing still separating them would be `owner_id`. Setting `lease_until = now` keeps the
row, keeps the token strictly monotone for the identity's whole life, and makes "immediately
claimable" true by the same single predicate `try_claim` already evaluates. Cardinality is
unaffected: one row per `(projection_id, tag, tenant)` ever seen — the same bound
`projection_offsets` has (AD-8).

**Error mapping**: `claim_error` reuses PROD-014B AD-8's `pub(crate) is_fatal`
(`postgres/mod.rs`) verbatim for the `Transient`/`Fatal` split, with one code checked first:
SQLSTATE `22003` (`numeric_value_out_of_range`) → `ClaimError::FencingExhausted`. Because the
increment happens **in SQL**, PostgreSQL raises rather than wraps at `BIGINT`'s ceiling, so this
adapter gets for free the guarantee `reservation.rs` has to buy with `token_for_storage`
(`:107-109`) — the deliberate difference between incrementing in SQL and incrementing in Rust.
The value read back through `RETURNING` still passes `token_from_storage` (AD-3).

**No `probe()`.** Same reasoning as PROD-014B AD-9: the port declares no health method, and an
unapplied migration `016` surfaces as `42P01` → `Fatal`.

### AD-6 — Where the claim wraps the batch, and the residual window, stated

**Decision** — `crates/domain/src/read_side/session.rs`:

```rust
/// Everything a session needs to claim its stream. One optional knob, so
/// every existing `ReadSideSession::new` call site compiles unchanged;
/// `Profile::Production` is what makes it non-optional in a real
/// composition (AD-9), not the type.
pub struct ReadSideClaiming {
    pub store: Arc<dyn ReadSideClaimStore>,
    pub owner: OwnerId,
    pub clock: Arc<dyn Clock>,
    pub lease: chrono::Duration,
}

impl<E, H, RS, DS, OS, PR> ReadSideSession<E, H, RS, DS, OS, PR> {
    pub fn with_claiming(mut self, claiming: ReadSideClaiming) -> Self { … }

    pub async fn execute(&self) -> Result<Option<Offset>, ProjectionError> {
        let Some(c) = &self.claiming else { return self.run_batch(None).await };
        let Some(fence) = c.store
            .try_claim(&self.claim_id(), &c.owner, c.clock.now() + c.lease)
            .await
            .map_err(…)?
        else {
            return Ok(None); // refused: no fetch, no handler (AD-4)
        };
        let result = self.run_batch(Some(&fence)).await;
        let _ = c.store.release(&fence).await; // best-effort; see below
        result
    }
}
```

`run_batch` is today's `execute()` body verbatim, plus one insertion between `handler.handle()`
and the commit loop:

```rust
if let (Some(c), Some(fence)) = (&self.claiming, fence) {
    c.store.renew(fence, c.clock.now() + c.lease).await.map_err(|e| match e {
        ClaimError::StaleOwner => ProjectionError::transient(
            "claim lost before commit; this batch's offset and dedup writes were \
             withheld so a replaced owner cannot rewind the current owner's offset",
        ),
        other => ProjectionError::transient(format!("claim renew failed: {other}")),
    })?;
}
```

**Criteria**:

1. **The refusal happens before `fetch`** (IS-4, Required Semantics line 1): a refused worker
   issues no `fetch` and reaches no handler, because `try_claim` is the first statement.
2. **Extraction into `run_batch` is what makes release unconditional.** Rust has no async
   `Drop`, so a scope guard cannot await. Splitting the body is the smallest construct that
   releases on the success path, both early-return paths (`events.is_empty()`,
   `unique_events.is_empty()`), and the handler-error path alike.
3. **A failed `release` is not a batch failure** and is deliberately swallowed: the work already
   committed, and the lease expires on its own. A failed `renew` is the opposite — it gates the
   write and must propagate.
4. **The `renew` is the fence gate, placed as late as possible.** It re-verifies the full triple
   in one statement and extends to a full fresh lease in the same statement, so the commit phase
   runs inside a freshly verified lease.

**The residual window, named rather than papered over.** Because `write_offset` and `mark_seen`
belong to two other ports that carry no fence (D-2) and share no transaction (D-11), the fence
gate and the writes it authorises are **adjacent, not atomic**. A worker is fenced out at the
gate — which is exactly what Required Semantics asks ("attempts to write … *as the owner*" is
refused, and the stored state is unmodified) — but a worker whose `renew` *succeeded* and whose
commit phase then outlives an entire freshly-granted lease could still land a late
`write_offset`. The bound is explicit and configurable: the commit phase (a `mark_seen` per event
plus one upsert, no user code) must exceed the whole lease duration. This is the same class of
statement `reservation.rs:13-34` makes about its own mechanism — "expiry decides when an attempt
is *permitted*, and fencing decides whose reservation outcome is *authoritative*. Neither makes two
concurrent executions impossible". Closing it fully requires one transaction spanning the claim,
the dedup markers, and the offset — which is D-11's excluded work, not this change's.

**Automatic background renewal during a long `handler.handle()` is not shipped.** `renew` is
delivered as a capability, which is what the Required Semantics asks ("MUST be **able to** extend
the lease"); the lease length is deployment configuration and a handler either finishes inside it
or is legitimately taken over. This is `operation/reservation.rs:19-27`'s decision verbatim —
"a deliberately deferred extension, not an oversight" — adopted here for the same reason.

**Owner identity is the host's obligation, and this must be documented on the knob.**
`ReadSideClaiming::owner` must be unique per *process instance*. Two live processes sharing one
`OwnerId` can each satisfy the other's fence `WHERE`, degrading execution exclusion to
lease-expiry alone. The port cannot verify this; the rustdoc states it and names the consequence.

### AD-7 — Scheduler: one optional `ProjectionSpec` knob, no trait signature change

**Decision** — `crates/runtime/src/read_side/scheduler.rs`:

```rust
impl<F, H, S, D, O, R> ProjectionSpec<F, H, S, D, O, R> {
    /// Claims each `(tag, tenant)` stream before processing it. Absent by
    /// default, exactly as `reporter`/`interval`/`on_error` are.
    pub fn claims(mut self, claiming: ReadSideClaiming) -> Self { … }
}
```

`spawn` already does `let mut scheduler = self;` (`:301`), so it moves `spec.claiming` onto a new
`TagSchedulerImpl` field before the loop; `start_projection` reads `self.claiming` and attaches
it to each session it constructs (`:90-100`).

**Criteria**: (a) `ProjectionSpec` already exists precisely to default optional knobs
(`:161-174`), so this is the established shape rather than a new one; (b) putting the config on
the *scheduler* rather than in the trait method means `TagScheduler::start_projection`'s public
signature — already seven parameters — is untouched, and no external implementor breaks;
(c) claim scope is one batch, so there is **no cross-tick claim state, no in-memory fence cache,
and no scheduler lifecycle to add** — each tick claims, works, and releases. Two extra statements
per stream per tick, never per event (R-3); (d) OOS-5/D-12 hold: `start_projection` stays the
sequential for-loop it is today.

### AD-8 — Migration `016_create_projection_claims.sql`

**Decision**:

```sql
-- Read-side processing claims: one row per (projection_id, tag, tenant),
-- naming the worker that currently holds single valid processing ownership
-- of that stream, until when, and under which fencing token.
--
-- IDENTITY — the primary key is byte-for-byte `projection_offsets`' identity
-- (013), which is the claim identity PROD-014C D-1 fixes. `tenant` is NOT NULL
-- for 013's reason: the read-side SPI's parameter is `tenant: &str`, never
-- `Option<&str>`, so there is no systemwide scope to model and no partial-index
-- pair is needed.
--
-- RELEASE IS AN EXPIRY, NOT A DELETE. A released claim is a row whose
-- `lease_until` has been set to the release instant, so `lease_until <= now`
-- is the single predicate meaning "claimable" for both released and lapsed
-- claims, and the fencing token stays strictly monotone for this identity's
-- whole life. Row count is therefore bounded by the number of streams ever
-- seen — the same bound `projection_offsets` has, and unlike
-- `projection_dedup` (014), which grows per event. No retention pass is
-- needed and none is shipped.
--
-- `claimed_at` is operational only; no decision reads it. Lease decisions are
-- made against the adapter's injected Clock, never the database's NOW()
-- (PROD-014C D-4).

CREATE TABLE IF NOT EXISTS projection_claims (
    projection_id VARCHAR(255) NOT NULL,
    tag           VARCHAR(255) NOT NULL,
    tenant        VARCHAR(255) NOT NULL,
    owner_id      VARCHAR(255) NOT NULL,
    fencing_token BIGINT       NOT NULL,
    lease_until   TIMESTAMPTZ  NOT NULL,
    claimed_at    TIMESTAMPTZ  NOT NULL DEFAULT NOW(),

    CONSTRAINT projection_claims_identity
        PRIMARY KEY (projection_id, tag, tenant),
    CONSTRAINT projection_claims_fencing_token_positive
        CHECK (fencing_token > 0)
);
```

**Criteria**: (a) `016` is the next free number — the sequence on `develop` ends at
`015_fix_aggregates_tenant_null_uniqueness.sql` (D-6); (b) the primary key **is** the unique
identity and **is** the only index every statement needs, all four being point lookups on it —
PROD-014B AD-1 criterion 1, and no surrogate `BIGSERIAL` because a `NOT NULL` tenant needs no
partial indexes; (c) **no index on `lease_until`**, deliberately: nothing scans by lease, because
there is no purge path and no queue; (d) `CHECK (fencing_token > 0)` mirrors `010`'s and is the
schema half of `token_from_storage`'s guard; (e) no `state` column — unlike
`operation_reservations` there is no terminal `completed` state to model, since a released claim
is an expired lease; (f) `VARCHAR(255)` continues the sequence, and an over-long identifier is
refused with `22001` → `Fatal`, never truncated; (g) registration is one `include_str!` const and
one appended entry in `migrations()`, and `migrations.rs`'s existing bidirectional registry test
fails if the file ships unregistered — no new migration test is written (PROD-014B AD-2).

### AD-9 — The Production gate reuses `require_durably_configured` verbatim

**Decision**: no new predicate. `crates/service-sdk/src/runtime/builder.rs` gains one slot and
one sibling validator, called from `validate_persistence_profile` after the existing two:

```rust
read_side_claims: Option<Arc<dyn ReadSideClaimStore + Send + Sync>>,

/// Under `Profile::Production`, a composition that registers read-side
/// progress must also register a durable claim store (PROD-014C IS-6).
///
/// The early return is INSIDE this function, never before the call: an
/// early return placed in `validate_persistence_profile` would skip this
/// check for every composition that registers no read-side progress AND
/// every one that does — PROD-014A EC-1's exact defect.
fn validate_read_side_claim_profile(&self) -> Result<(), RuntimeError> {
    // Registration of a progress pair is the composition-visible signal that
    // this application processes a read side at all. A command-only service
    // is never forced to register a claim store it would never use
    // (PROD-014A IS-5, and `validate_effect_store_profile`'s own shape).
    if self.read_side_progress.is_empty() {
        return Ok(());
    }
    persistent_entity::profile::require_durably_configured(
        self.profile,
        self.read_side_claims.as_ref().is_some_and(|c| c.is_durable()),
        "durable read-side claim store (ReadSideClaimStore)",
        "AppBuilder::read_side_claims(store) (or \
         RuntimeBuilder::with_read_side_claim_store(..)), passing a store whose \
         is_durable() returns true",
    )?;
    Ok(())
}
```

**Criteria**: (a) D-5 asks whether the gate reuses `require_durably_configured` verbatim — it
does, unchanged, because the predicate's whole job is `Production && !durably_configured ⇒
refuse` and that is exactly this rule; (b) the `is_some_and(|c| c.is_durable())` argument is the
identical idiom `validate_effect_store_profile` already uses for the effect store
(`builder.rs:863-865`), and it is the composition of *presence* and *durability* that
`profile.rs:41-50` warns must never collapse into `.is_some()`; (c) **one global slot, not a
per-projection map**: `projection_id` is part of the claim identity, so one store serves every
projection — unlike a progress pair, which is inherently per-projection; (d) registration mirrors
`effect_store`'s split — last-write-wins on `RuntimeBuilder`, fail-closed duplicate guard on
`AppBuilder` (`CompositionError::DuplicateReadSideClaimStore`), which is PROD-014A's established
division and avoids the second parallel check.

**Post-change posture (IS-6)**: multi-replica read-side becomes **supported under an explicit
operational constraint** — a durable claim store registered, per-process-unique `OwnerId`s, and
handler effects still at-least-once (D-8, OOS-2).

### AD-10 — Tests: unit for shape and gate, real PostgreSQL for every contention claim

**Decision** per D-7: an atomicity claim is a claim about what the database does under real
contention, so no unit test simulates it. Strict TDD — the contention suite is written RED
against types that do not compile yet.

| Level | Location | What it proves |
|---|---|---|
| Unit | `persistence-api/src/read_side/claim.rs` `#[cfg(test)]` | AD-3: `Arc<T>` forwards `is_durable()` (the PROD-014A EC-2 landmine) and all three methods; `ClaimId`/`ClaimFence` identity |
| Unit | `domain/src/read_side/session.rs` `#[cfg(test)]`, scripted doubles, **no pool** | AD-6 ordering: a refused `try_claim` ⇒ `fetch` never called, handler never invoked, `Ok(None)`; a `renew` returning `StaleOwner` ⇒ **no** `mark_seen` and **no** `write_offset`, error propagates; `release` is called on the success path, both empty-early-return paths, and the handler-error path |
| Unit | `service-sdk/src/runtime/builder.rs` `#[cfg(test)]` | AD-9 matrix: Production + progress registered + no claim store ⇒ refuse naming capability and fix; + volatile claim store ⇒ refuse; + durable ⇒ ok; Production + **zero** progress registered + no claim store ⇒ ok (the EC-1-shaped early-return test); Dev + nothing ⇒ ok; `build()` and `try_build()` agree |
| Unit | `persistence/src/postgres/migrations.rs` (existing tests, no new code) | `016` is registered and ordered |
| Integration (real PG) | `integration-tests/tests/infrastructure/read_side_claiming_postgres.rs` | SC-1, SC-2, SC-3, SC-5, SC-7 — below |

Harness, copied from `takeover_fencing_postgres.rs` and `concurrent_replicas_postgres.rs`:
`isolated_database()` per test; **separate `PgPoolOptions` pools per contender** so they share no
connection; `SettableClock` moved by hand so expiry is reached without sleeping;
`tokio::sync::Barrier` releasing both attempts together; `AtomicUsize` observers that record
without participating; every wait bounded by a `WAIT_LIMIT` assertion rather than a silent
timeout; final state read back with raw `sqlx::query_as` **never through the port under test**;
`db.close().await` at the end.

- **SC-1 — execution exclusion.** Two workers, two pools, two `OwnerId`s, released together onto
  the same `(projection_id, tag, tenant)`: exactly one gets `Some(fence)`, the other `None`; the
  refused one's `fetch` counter and handler counter are both **0**. *Control case*, per
  `concurrent_replicas_postgres.rs:581-592`'s own: the same two workers on two different tenants
  both obtain a fence and both run — without it, a harness that refused whatever arrived second
  would satisfy every assertion above while proving nothing.
- **SC-2 — takeover without operator action.** A claims and never releases (its session is
  dropped mid-batch — the modelled death); the clock is advanced past `lease_until`; B's
  `try_claim` returns `Some`, with `fencing_token` **strictly greater**, and the row's `owner_id`
  is B's.
- **SC-3 — stale owner cannot write as owner.** After B's takeover: `renew(a_fence)` and
  `release(a_fence)` are both `Err(StaleOwner)` and the row still holds B's owner and B's token,
  unchanged. Then, at session level: A's batch is driven through its commit phase with the stale
  fence, and `projection_offsets` is read back through raw SQL and **still holds B's value** —
  the rewind did not happen. Plus `takeover_fencing_postgres.rs:182-213`'s token-isolation probe:
  **B's owner with A's stale token** must also be refused, so the refusal cannot be attributed to
  `owner_id` alone.
- **SC-5 — ordering unchanged.** One worker holds the claim across a batch of at least three
  events; the handler's received slice is asserted strictly ascending by `event_version`.
- **Mutation checks, measured rather than assumed** (the habit `reservation.rs:286-298` and
  `takeover_fencing_postgres.rs:182-196` establish): deleting `AND projection_claims.lease_until
  <= $6` from `try_claim`'s `ON CONFLICT` `WHERE` must make SC-1 fail with both workers claiming;
  deleting `AND fencing_token = $6` from the shared fence `WHERE` must make SC-3's token probe
  fail. Both are recorded in the suite's module doc.

Two properties are diff properties, checked by reading the change rather than by a test:
**SC-6** (no delivered artifact says exactly-once — the grep gate R-1 names) and the
untouched-file list in the Component Map (`read_side_offset.rs`, `read_side_dedup.rs`,
`offset.rs`, `dedup.rs`).

---

## Integration Points

| Boundary | Direction | Mechanism | Verified at |
|---|---|---|---|
| `persistence-api` → `ego-domain` | up | existing module re-export block | `domain/src/read_side/mod.rs:21-24` |
| port → `Arc<dyn …>` erasure | out | new blanket impl (AD-3) | mirrors `offset.rs:91-119` |
| `is_durable()` → `Profile::Production` | in | `require_durably_configured`, unchanged | `profile.rs:51-63`; AD-9 |
| `AppBuilder` → `RuntimeBuilder` | down | thin delegation + dup guard | mirrors `app/mod.rs:590-624` |
| `ProjectionSpec` → session | out | one optional knob, moved through `spawn` | `scheduler.rs:288-301`; AD-7 |
| schema → runtime | in | existing `include_str!` + `raw_sql` runner | `migrations.rs`; AD-8 |
| adapter → `Clock` | in | `Arc<dyn Clock>` by constructor | `reservation.rs:85-87`; D-4 |
| claim port → `OffsetStore`/`DedupStore` | **none** | no path added, none exists | D-2, D-11 |

## Threat Matrix

N/A — no routing, shell command, subprocess, VCS/PR automation, executable-file classification,
or process-integration boundary. This change adds one SPI, one SQL adapter, one DDL file, one
composition gate, and one test suite; no external process is invoked and no file is executed or
classified.

The applicable surface is `ego-rs-security` Rules 1 and 2, closed by construction: every value is
a bound `$N` and nothing — identifier or value — is interpolated into SQL text (AD-5).
`projection_claims` **is** tenant-scoped data and binds `tenant` in every statement, so
PROD-014B AD-7's `projection_dedup` carve-out does not apply here and is not reused.

## Migration / Rollout

One additive `CREATE TABLE IF NOT EXISTS`, referenced by nothing else and referencing nothing
else. No existing table, column, index, or query changes. Deploy order is the existing one:
`migrations::run` already precedes every store construction.

Rolling deploy is safe in both directions and worth stating, because it is the deployment this
change exists for. New replicas claim; old replicas do not. During the overlap an old replica can
still process a stream a new one holds — which is today's behaviour, not a regression introduced
by the partial rollout — and the guarantee becomes effective once the last old replica is gone.
Under `Profile::Production` the fleet cannot start *without* a claim store once this ships, so
the overlap is bounded by the rollout itself.

Rollback is the proposal's: delete the port, adapter, re-export and gate; drop `016` and its
registry entry; remove the `ProjectionSpec` knob and the `with_claiming` call; restore the
single-writer adoption constraint in specs and docs. The table may be dropped or left — nothing
else references it, and discarding it degrades behaviour to PROD-014B's, not to corruption.

## Traceability

| Proposal item | Resolved by |
|---|---|
| D-1, IS-1 | AD-2 (`ClaimId`), AD-8 (PK identical to `013`'s) |
| D-2, IS-2 | AD-1, AD-3 — new port, `Arc<T>` forwarding, `OffsetStore`/`DedupStore` untouched |
| D-3, IS-3, (D) | AD-2, AD-5, AD-6 — token reused, minted in SQL, verified in every `WHERE` |
| D-4, R-5 | AD-5 (adapter's injected `Clock`), AD-6 (`ReadSideClaiming::clock`) |
| D-5, IS-6, SC-4 | AD-9 — `require_durably_configured` verbatim, early return inside the function |
| D-6, IS-5 | AD-8 — `016`, one row per stream, no retention |
| D-7, IS-7, SC-7 | AD-10 — `isolated_database()`, separate pools, no unit-level simulation |
| D-8, OOS-2, R-1, SC-6 | Wording note at the head; AD-6's residual-window statement; SC-6 is a diff property |
| D-9 | AD-5 — one row, one statement; no consensus, no leader election, no broker |
| D-10, OOS-3 | Untouched: no retry/backoff added |
| D-11, OOS-4 | AD-6 — named as the residual window's cause, not closed |
| D-12, OOS-5 | AD-7(d) — `start_projection` stays sequential |
| IS-4, SC-1 | AD-6 — `try_claim` is the first statement; refusal precedes `fetch` |
| SC-2 | AD-5 takeover path; AD-10 SC-2 |
| SC-3 | AD-5 fence `WHERE`; AD-6 pre-commit gate; AD-10 SC-3 |
| SC-5 | AD-7(c) claim is per stream; AD-10 SC-5 |
| R-2 | AD-6 — `renew` capability, lease as configuration, fencing rejects the evicted writer |
| R-3 | AD-7(c) — per stream per batch, never per event |
| R-4 | `sdd-tasks` owns the 400-line forecast; not pre-empted here |
| Approach's open question | The A–E comparison: (C) chosen, three methods not four |

## Open Questions

- [ ] AD-6 — the fence gate and the writes it authorises are adjacent, not atomic, because
      D-2 and D-11 are both binding. Confirm the stated bound (a commit phase outliving a full
      freshly-granted lease) is acceptable, rather than promoting the shared-transaction work
      D-11 excluded.
- [ ] AD-6 — `OwnerId` per-process uniqueness is a host obligation the port cannot verify.
      Confirm documenting it on the knob is sufficient, rather than the framework deriving an
      owner id itself.
- [ ] AD-9 — the gate keys off "a progress pair is registered". A projection spawned directly
      through `ProjectionSpec`/`TagSchedulerImpl` without passing the composition root stays
      ungoverned, exactly as PROD-014A OOS-7 already establishes. Confirm that boundary is
      intentionally unchanged here.
