# Design: STOOLAP-RS-01 — Durable Read-Side Stores on Stoolap

> Canonical / English. Spanish companion: `design.es.md` (1:1 headings and decision IDs).

## Technical Approach

Three Stoolap-backed stores behind one new `read-side` Cargo feature on the existing
`crates/persistence-stoolap` crate, each implementing one already-shipped port from
`crates/persistence-api/src/read_side/` with no contract change:

- `StoolapOffsetStore` — `read_offset`/`write_offset`, last-write-wins, keyed
  `(projection_id, tag, tenant)`.
- `StoolapDedupStore` — `seen`/`mark_seen`, keyed `(projection_id, tag, event_id)`, no tenant.
- `StoolapReadSideClaimStore` — `try_claim`/`renew`/`release`, keyed
  `ClaimId { projection_id, tag, tenant }`, fenced by `FencingToken`.

The reference implementation for the *algorithm* is `StoolapOperationReservationStore`
(`crates/persistence-stoolap/src/operation/reservation.rs`), not the Postgres read-side stores:
only shapes already proven against Stoolap 0.4 in this repository are used. The reference for the
*port semantics* is `crates/persistence/src/postgres/read_side_{offset,dedup,claim}.rs`.

Spec: `persistence-stoolap-read-side` (new capability). No existing spec is modified — the
`Profile::Production` gates (`builder.rs:906-946`) already exist and are satisfied, not changed.

## Architecture Decisions

### AD-1: A new `read-side` feature on the existing crate, not a new crate

**Choice**: `read-side = ["dep:tokio", "dep:async-trait", "dep:chrono", "dep:ego-domain"]` in
`crates/persistence-stoolap/Cargo.toml`, alongside the existing `event-sourcing` (`:52`) and
`operation-reservation` (`:56-62`) features.

**Alternatives considered**: a new `ego-persistence-stoolap-read-side` crate; reusing
`operation-reservation`.

**Rationale**: All four optional deps are **already declared** on that manifest (`chrono` `:16`,
`tokio` `:17`, `async-trait` `:18`, `ego-domain` `:21`) — zero manifest additions, zero new
transitive edges, exactly as the proposal states. `base64` is deliberately *not* included: no
read-side column stores bytes. Reusing `operation-reservation` would hand every read-side consumer
the `base64` dependency and the reservation store itself, defeating the reason AD-7 of STOOLAP-S3
created a separate feature in the first place. A new crate would duplicate `stoolap_common`,
`dsn_for` and the `sync=full` fail-closed check for three small stores.

`crates/service-sdk/Cargo.toml:75-77` already dev-depends on this crate with
`features = ["operation-reservation"]`; that list gains `"read-side"` (AD-12).

### AD-2: `src/read_side/{mod,offset,dedup,claim}.rs`; three independent `open()`s, no bundle type

**Choice**: `src/read_side/mod.rs` declaring `pub mod offset; pub mod dedup; pub mod claim;`,
gated in `lib.rs` by `#[cfg(feature = "read-side")] pub mod read_side;` with crate-root
`pub use` of the three store types — the identical shape `operation/` already has
(`lib.rs:13-14,22-23`). Each store has its own
`open(path: &Path, …) -> Result<Self, PortError>`. **No** `StoolapReadSideStores` bundle factory.

**Alternatives considered**: one `read_side.rs` file; a `StoolapReadSideStores::open(path)`
factory returning the trio, mirroring `reference_app::read_side::ReadSideProgressStores::postgres`.

**Rationale**: One file per port matches how both the port crate
(`persistence-api/src/read_side/{offset,dedup,claim}.rs`) and the Postgres adapter
(`persistence/src/postgres/read_side_{offset,dedup,claim}.rs`) already file this capability. The
bundle factory is a type with one caller (the composition test) that the gate does not want anyway
— it consumes three separate `Arc`s. Three `open()` calls at one path is also *stronger* evidence:
it exercises the shared-engine property (STOOLAP-S3 design.md "Concurrency Scope") instead of
hiding it. Trigger to revisit: a host outside this repo asks for it.

### AD-3: Minimal Stoolap-native schema — `UNIQUE`, no `PRIMARY KEY`, no `CHECK`, no timestamps beyond the lease

**Choice**: three tables created by their own store's `open()`, named after the Postgres tables so
the logical schema reads the same across backends.

```sql
CREATE TABLE IF NOT EXISTS projection_offsets (
    projection_id TEXT    NOT NULL,
    tag           TEXT    NOT NULL,
    tenant        TEXT    NOT NULL,
    offset_value  INTEGER NOT NULL,        -- i64; Offset has exactly one variant, Sequence(i64)
    UNIQUE (projection_id, tag, tenant)
);

CREATE TABLE IF NOT EXISTS projection_dedup (
    projection_id TEXT NOT NULL,
    tag           TEXT NOT NULL,
    event_id      TEXT NOT NULL,           -- no tenant column: the port takes no tenant
    UNIQUE (projection_id, tag, event_id)
);

CREATE TABLE IF NOT EXISTS projection_claims (
    projection_id TEXT      NOT NULL,
    tag           TEXT      NOT NULL,
    tenant        TEXT      NOT NULL,
    owner_id      TEXT      NOT NULL,
    fencing_token INTEGER   NOT NULL,      -- i64, always >= 1
    lease_until   TIMESTAMP NOT NULL,
    UNIQUE (projection_id, tag, tenant)
);
```

**Rationale, point by point**:

| Choice | Why |
|---|---|
| `UNIQUE`, never `PRIMARY KEY` | Stoolap 0.4 **parses but does not enforce** a table-level composite `PRIMARY KEY` (no constraint, no index); `UNIQUE` is enforced and is what `ON CONFLICT` matches against. Confirmed and repeated in every Stoolap store in-tree: `reservation.rs:64,165-168`, `event_store.rs:50,63`, `repository.rs:16`, `snapshot.rs:31`, `effect-store/src/stoolap/mod.rs:230-236` |
| No `CHECK (fencing_token > 0)` | No Stoolap table in this codebase uses a `CHECK`. The invariant is enforced in Rust by `FencingToken` itself plus `token_from_storage`'s `raw <= 0` rejection (AD-11) — one definition, not two |
| No triggers, no stored procedures | None exist anywhere in this codebase, on either backend |
| Dedup has no `seen_at` column | Pruning/TTL/retention is an explicit spec Non-Goal; an unread column is dead weight and would invite a retention feature the spec forbids |
| `tenant` stored verbatim; `stoolap_common::encode_tenant` **not** used | The read-side ports take `tenant: &str`, not `Option<TenantId>` — there is no absent-tenant case to encode. Postgres binds it directly too (`read_side_offset.rs:79`). Using the `""` sentinel here would invent a distinction the port does not have |
| `lease_until TIMESTAMP` | Same column type and read path as `reservation.rs:60`; no `FromValue for DateTime<Utc>` exists in Stoolap 0.4, so it is read via `row.get_value(idx)` matched against `Value::Timestamp` (`reservation.rs:117-128`) |

### AD-4: Offset and dedup writes use only statement shapes already proven against Stoolap 0.4

**Choice**:

- `mark_seen`: one statement, `INSERT INTO projection_dedup (…) VALUES ($1,$2,$3) ON CONFLICT
  (projection_id, tag, event_id) DO NOTHING`. Idempotent by construction; a repeat affects zero
  rows and is **not** an error. `seen`: `SELECT 1 … LIMIT 1`, presence is the answer.
- `write_offset`: **UPDATE-first**, then insert-if-absent, then re-UPDATE only if a racer inserted:

```
1. UPDATE projection_offsets SET offset_value=$v WHERE projection_id=$p AND tag=$t AND tenant=$tn
   affected >= 1  -> done                                  (steady state: ONE statement)
2. INSERT INTO projection_offsets (…) VALUES (…)
     ON CONFLICT (projection_id, tag, tenant) DO NOTHING
   inserted == 1  -> done                                  (first write for this key)
3. else a concurrent writer inserted in the window -> repeat statement 1, done
```

**Alternatives considered**: `INSERT … ON CONFLICT … DO UPDATE SET offset_value = EXCLUDED.…`,
which is what Postgres uses (`read_side_offset.rs:94-99`).

**Rationale**: `ON CONFLICT … DO UPDATE` is used **nowhere** against Stoolap in this repository —
every Stoolap upsert in-tree is either `DO NOTHING` (`reservation.rs:256`,
`effect-store/src/stoolap/mod.rs:530,775`) or an explicit `SELECT`-then-`INSERT`-or-`UPDATE`
(`snapshot.rs`). Its behaviour on Stoolap 0.4 is unproven, and the proposal's own risk row says
port the proven shape rather than the Postgres one. UPDATE-first is chosen over
INSERT-then-UPDATE because offset writes are the per-batch hot path and the row exists for every
write after the first, so the steady state costs one statement, not two. Correctness under the
port's **last-write-wins** contract is unaffected by which racer's value survives — the trait
expresses no compare-and-swap and the spec forbids adding one.

### AD-5: `try_claim` ports the reservation store's two-statement CAS

**Choice** (`now = clock.now()`, `lease_until` is the caller's parameter):

```
1. INSERT INTO projection_claims (projection_id, tag, tenant, owner_id, fencing_token, lease_until)
   VALUES ($p, $t, $tn, $owner, 1, $lease_until)
   ON CONFLICT (projection_id, tag, tenant) DO NOTHING
   inserted == 1  ->  Ok(Some(fence{ FencingToken::initial() }))          # fresh grant

2. SELECT owner_id, fencing_token, lease_until FROM projection_claims WHERE $p AND $t AND $tn
   no row         ->  Err(Transient("claim row vanished after the insert conflict; retry"))

3. now < row.lease_until  ->  Ok(None)                                    # live lease: refusal

4. displaced = token_from_storage(row.fencing_token)?
   next      = displaced.next().ok_or(ClaimError::FencingExhausted)?
   UPDATE projection_claims
      SET owner_id=$owner, fencing_token=$next, lease_until=$lease_until
    WHERE projection_id=$p AND tag=$t AND tenant=$tn
      AND fencing_token = $displaced      -- CAS against the token step 2 observed
      AND lease_until  <= $now            -- re-verified against the LIVE row, not the read
   affected == 1  ->  Ok(Some(fence{ next }))                             # takeover

5. affected == 0  ->  Ok(None)            # a peer took over or the owner renewed in the window
```

**Alternatives considered**: Postgres's single-statement
`INSERT … ON CONFLICT … DO UPDATE … WHERE projection_claims.lease_until <= $now RETURNING
fencing_token` (`read_side_claim.rs:176-195`).

**Rationale**: The Postgres form depends on three things unproven against Stoolap 0.4 —
`DO UPDATE`, a `WHERE` clause attached to `DO UPDATE`, and `RETURNING`. The two-statement form is
already stress-tested for intra-process concurrency in this crate
(`tests/reservation_conformance.rs:192-245`).

**Why no lost update is possible.** Statement 4's `WHERE` re-asserts *both* values step 2 read.
Two tasks that concurrently observe `fencing_token = 5, lease_until <= now` both compute
`next = 6`, but at most one `UPDATE` can commit against that row version under Stoolap's MVCC. The
loser resolves one of exactly two ways, and never a third:

| Loser observes | Classified as | Why it is correct |
|---|---|---|
| `affected == 0` (committed token is now 6, or the owner renewed so `lease_until > now`) | `Ok(None)` | A live lease holds the claim. A refusal, not a failure — precisely the port's contract |
| a raw error for which `stoolap_common::is_write_conflict` is `true` (`UniqueConstraint`, `TransactionAborted`, `LockAcquisitionFailed`, `DatabaseLocked`, the pinned `"uncommitted changes from transaction"` message) | `ClaimError::Transient(msg)` | The statement did not run to a verdict; retry is safe and says nothing about ownership |

Statement 1 races resolve the same way: on an empty table exactly one `INSERT` succeeds (`UNIQUE`
plus `DO NOTHING` means the loser sees `0`, not a violation), the loser falls to step 2, reads the
winner's fresh row and refuses at step 3. Exactly one `Ok(Some(fence))` per live lease, always.

Unlike `reserve`, step 5 needs **no** re-read: the claim port has no `OwnedInProgress` /
`OtherInProgress` distinction to recover — `Ok(None)` is the whole answer.

### AD-6: `renew` and `release` are one statement and one concrete helper; **no** shared `mutate_owned` combinator

**Choice**: one private `fn set_lease(&self, fence, new_lease_until) -> Result<(), ClaimError>`
inside `read_side/claim.rs`, issuing the single statement below; `renew` calls it with the
caller's `lease_until`, `release` calls it with `now`. `affected == 0` ⇒ `ClaimError::StaleOwner`.

```sql
UPDATE projection_claims SET lease_until = $1
 WHERE projection_id = $2 AND tag = $3 AND tenant = $4
   AND owner_id = $5 AND fencing_token = $6
   AND lease_until > $7            -- $7 = now; a lapsed holder may not resurrect its claim
```

**Alternatives considered**: (i) inline the statement twice, as `reservation.rs:382-478` does for
its three mutators; (ii) promote a generic cross-store `mutate_owned<F>` combinator now that this
crate would have five fence-verified mutators, which `reservation.rs:382-387`'s own ponytail
comment names as the trigger ("promote … if a fourth mutator with the same ownership predicate
shows up").

**Rationale — the threshold comment is answered explicitly, not left open.** A *cross-store*
combinator is rejected: the two modules differ in error type (`ReservationError` vs `ClaimError`),
key arity (2 columns vs 3), and extra predicate (`state='in_progress'` vs none), so a shared
generic would need an error mapper, a key tuple and a predicate fragment as parameters — more code
at each of five call sites than the five statements it replaces. `reservation.rs`'s three
mutators therefore stay inline, untouched by this change. Inside `claim.rs` the case is different
and stronger than "similar": `renew` and `release` are the **identical statement** with one
different bound value — Postgres's own `read_side_claim.rs:210-254` binds `lease_until` in one and
`now` in the other against a byte-identical query. One concrete 15-line helper, not a generic
combinator.

**Release is an expiry, never a `DELETE`**: setting `lease_until = now` keeps the row, so the
fencing token stays strictly monotone across the release boundary and the claim is immediately
reclaimable — the requirement the spec states and the rule Postgres's module doc
(`read_side_claim.rs:20-25`) already carries.

### AD-7: The claim store owns an injected `Clock`; the caller owns `lease_until`

**Choice**: `StoolapReadSideClaimStore::open(path: &Path, clock: Arc<dyn ego_domain::Clock>)`.
Every expiry comparison reads `clock.now()`; the store never calls `Utc::now()`,
`SystemTime::now()`, or a SQL `now()`. `lease_until` always arrives as a `DateTime<Utc>` parameter
on `try_claim`/`renew` and is stored verbatim. `StoolapOffsetStore` and `StoolapDedupStore` take
no clock at all — neither port has a time dimension.

**Rationale**: This is the shape both existing implementations of this exact port family already
have — `PostgreSQLReadSideClaimStore { pool, clock }` (`read_side_claim.rs:83-86,105-107,163`) and
`StoolapOperationReservationStore { db, clock }` (`reservation.rs:132-135,153,308`) — and it is
the only shape the port's signature permits: `try_claim(claim_id, owner_id, lease_until)` carries
no `now` parameter, so "is the incumbent lease still live?" is unanswerable without a clock the
store holds. Determinism is preserved because the clock is injected by the composition root: tests
drive `ego_testkit::TestClock` and advance it explicitly, exactly as
`reservation_conformance.rs:114` does.

**Spec-wording reconciliation, stated rather than glossed.** The spec's "Lease Expiry Is Always
Caller-Computed" scenario reads "never on a clock read performed inside the store". Taken
literally that is unimplementable against the unmodified trait, and no in-tree implementation of
any leased port satisfies it. The satisfiable reading — and the one this design implements — is:
the *lease bound* is always computed by the caller and never by the store, and the store's own
"now" comes from an injected, caller-supplied `Clock`, never from ambient system time. Flagged as
a risk below so `sdd-verify` reads the requirement the same way rather than failing it.

### AD-8: Errors classify `Transient` vs `Fatal` through `is_write_conflict`

**Choice**: for all three stores, a raw `stoolap::Error` for which
`stoolap_common::is_write_conflict` returns `true` maps to the port's `Transient` variant;
everything else maps to `Fatal`. `affected == 0` on a fence-verified mutation maps to `StaleOwner`
(AD-6), never to `Transient`.

**Rationale**: `OffsetStoreError`, `DedupStoreError` and `ClaimError` each have both a `Transient`
and a `Fatal` variant, so these three stores can classify honestly — unlike
`OperationReservationStore`, whose port has neither, forcing `reservation.rs` to smuggle the retry
hint into `Backend("…retry")` (STOOLAP-S3 AD-5). Default is fail-loud: `is_write_conflict`'s own
wildcard arm keeps anything unrecognised out of `Transient`. This is what makes the spec's
"`Ok(None)` **or** a retry-safe `Transient`" concurrency clause literally satisfiable.

### AD-9: `open()` fails closed on `sync=full`; `is_durable()` re-derives from the live DSN

**Choice**: each store's `open()` builds its DSN with `stoolap_common::dsn_for(path)`, opens, and
refuses with the port's `Fatal` variant if `dsn_declares_sync_full(db.dsn())` is false, before
issuing `CREATE TABLE`. `is_durable()` returns `dsn_declares_sync_full(self.db.dsn())` — not a
hardcoded `true`.

**Rationale**: Verbatim the pattern `reservation.rs:153-173,235-237`, `snapshot.rs` and
`event_store.rs` already share (STOOLAP-S3 AD-9). A hardcoded `true` would let `is_durable()`
outlive-lie about how the store was opened, which is exactly what the `Profile::Production` gate
reads.

### AD-10: `run_blocking` stays duplicated per store; it is **not** centralised in `stoolap_common`

**Choice**: each of the three new stores carries its own private
`async fn run_blocking<F, R>(&self, f: F) -> Result<R, PortError>` handing the closure to
`tokio::task::spawn_blocking` over a cloned `Database` — never `block_in_place`.

**Alternatives considered**: promote a single generic helper into `stoolap_common` now that six
stores would use it (`effect-store/src/stoolap/mod.rs:277`, `event_store.rs:236`,
`reservation.rs:177`, plus these three).

**Rationale**: The genuinely shared part is four lines
(`spawn_blocking(move || f(&db)).await`); the rest is the per-port mapping of a `JoinError` to
that port's own failure variant, and there are six different ones. A centralised version needs an
`on_panic: impl FnOnce(String) -> E` parameter threaded through roughly thirty call sites — more
code added at the call sites than removed from the definitions, and it would edit three shipped,
otherwise-untouched stores, spending review budget this change has better uses for.
`stoolap_common` exists for things that are *identical* across stores (`dsn_for`, `encode_tenant`,
`is_write_conflict`, `dsn_declares_sync_full`), which this is not. `block_in_place` remains
forbidden: it panics outside a multi-threaded runtime and would break the current-thread
`#[tokio::test]`s (`reservation.rs:14-19`).

### AD-11: `token_for_storage` / `token_from_storage` move into `stoolap_common` as `pub(crate)`

**Choice**: lift the two i64↔`FencingToken` guards out of `operation/reservation.rs:76-93` into
`persistence/stoolap_common.rs` as `pub(crate)`, and have both `operation/reservation.rs` and
`read_side/claim.rs` import them. `read_side/claim.rs` adds a
`to_claim_error(ReservationError) -> ClaimError` shim, mirroring
`crates/persistence/src/postgres/read_side_claim.rs:51-57` verbatim.

**Alternatives considered**: (i) bump the two functions to `pub(crate)` **in place**, the literal
mirror of what Postgres did (`postgres/reservation.rs:107,124` are `pub(crate)` precisely so
`read_side_claim.rs:40` can reuse them); (ii) duplicate them a third time inside `read_side/`,
which is what STOOLAP-S3 AD-10 did when the boundary was cross-crate.

**Rationale**: The Postgres reuse works because both modules are unconditionally compiled. Here
they are not — `operation/reservation.rs` sits behind `#[cfg(feature = "operation-reservation")]`
(`lib.rs:13`), so an in-place `pub(crate)` reference from `read_side` breaks under
`--features read-side` alone, and repairing it by making `read-side` imply
`operation-reservation` would hand every read-side consumer the `base64` dependency and the
reservation store (against AD-1). Hoisting solves it with no feature coupling and no third copy:
both types the guards touch (`FencingToken`, `ReservationError`) live in `ego-persistence-api`
(`persistence-api/src/operation/reservation.rs:300,457`), an **unconditional** dependency, so
`stoolap_common` — itself created to end exactly this kind of per-store duplication
(`stoolap_common.rs:1-5`) — can host them with no new edge. Diff: two functions moved, one import
line changed in a shipped file.

### AD-12: Stoolap-local tests only — **no** shared `ego-testkit` conformance harnesses (resolves Open Question 1)

**Choice**: verify all three stores with tests colocated in the modules plus one integration
binary `crates/persistence-stoolap/tests/read_side_stores.rs`
(`required-features = ["read-side"]`). Do **not** add
`assert_offset_store_conformance` / `assert_dedup_store_conformance` /
`assert_claim_store_conformance` to `crates/testkit/src/lib.rs`.

**Rationale — sized to the behaviour count, not to symmetry**: `OffsetStore` and `DedupStore` are
two-method ports with about five behaviours between them (absent reads `None`/`false`, per-key
isolation, last-write-wins, mark idempotence, no pruning). A shared harness for five assertions is
more scaffolding — factory-closure generics, a module, exports, doc comments — than the assertions
it carries. `ReadSideClaimStore` genuinely would benefit from one (seven behaviours, two
independent durable implementations that must agree), **but a harness is only worth its cost once
a second backend runs through it**, and routing `PostgreSQLReadSideClaimStore` through it means
touching Postgres, which this change's own Non-Goals forbid. A harness with exactly one caller is
the one-implementation interface this repository's review discipline rejects. The existing
precedent supports the sizing rather than contradicting it:
`assert_reservation_store_conformance` was justified by a seven-method port with three
implementations, and it was added in the change that created the *second* one.

**Named follow-up, not silence**: `assert_claim_store_conformance` in `ego-testkit`, driving both
`PostgreSQLReadSideClaimStore` and `StoolapReadSideClaimStore`. Its trigger is a third claim
backend or the first cross-backend divergence — not this change.

### AD-13: The composition test lives in `crates/service-sdk/tests/`, not `integration-tests/` and not `persistence-stoolap/tests/` (resolves Open Question 2)

**Choice**: extend `crates/service-sdk/tests/read_side_progress_composition.rs` with the real
`Profile::Production` composition and its volatile negative control, over a `tempfile` directory.
`crates/service-sdk/Cargo.toml:75-77`'s existing dev-dependency feature list gains `"read-side"`.

**Alternatives considered**: (i) `integration-tests/`, the Postgres precedent
(`integration-tests/tests/infrastructure/read_side_progress_postgres.rs:377-451`);
(ii) `crates/persistence-stoolap/tests/`, as the proposal hypothesised.

**Rationale**: Option (ii) is not merely heavier, it is **not available**: `ego-service-sdk`
depends on `ego-persistence-stoolap` (`service-sdk/Cargo.toml:75`), so a dev-dependency in the
other direction closes a cycle — and this manifest twice documents refusing exactly that
(`persistence-stoolap/Cargo.toml:28-29,39-41`: *"One direction only … so no cycle"*). Without
`RuntimeBuilder`/`App`, a test in that crate could only assert `is_durable()` on an isolated
store, which the spec explicitly says is **not** sufficient. Option (i) carries the Postgres
precedent's whole cost for none of its reason: `integration-tests/` is a separate workspace with a
container-provisioned database, a run-suite binary and a ledger admission contract (AD-14), all of
which exist because PostgreSQL needs external infrastructure. Stoolap is embedded and file-backed;
a `tempfile::tempdir()` is the entire infrastructure.

Option (iii), chosen, is not a compromise but the established in-tree precedent for this exact
problem: STOOLAP-S3 solved it identically with
`crates/service-sdk/tests/operation_reservation_gate_composition.rs`, whose header states the same
purpose ("cross-backend gate proof … the workspace's REAL implementations … not a synthetic
stub") and whose task 5.3 already opens a real Stoolap store over a `tempdir` and drives
`App::builder()`. Cost of this decision: one word in one dev-dependency feature list.

### AD-14: No delta to `real-infrastructure-verification` (resolves Open Question 3)

**Choice**: this change adds no requirement to
`openspec/specs/real-infrastructure-verification/spec.md`.

**Rationale, read from that spec**: its Purpose scopes it to *"which invariants MUST be
demonstrated against real PostgreSQL … and the wall-clock budget"*, and all five requirements are
PostgreSQL-specific (the two Postgres conformance harnesses, migration 007's backfill, the
`integration-tests/` admission ledger, PG14/PG16 version compatibility). Its Non-Goals are
PostgreSQL-infrastructure concerns. This change provisions no infrastructure, adds no file to
`integration-tests/`, and spends none of that suite's budget, so no requirement there is engaged.
Adding a Stoolap requirement would widen a PostgreSQL-*methodology* capability into a general one
— a second place where durable-backend verification is defined, the same trap STOOLAP-S3 AD-9
avoided. The precedent is decisive: STOOLAP-S2 and STOOLAP-S3 both shipped real Stoolap durability
**and** real Production-composition tests, and neither added a delta there (their delta sets are
`persistence-stoolap-event-sourcing` and `{persistence-api-surface, persistence-memory-adapter,
idempotent-command-processing, persistence-stoolap-operation-reservation,
production-composition-hardening}` respectively). This change's own composition requirement is
already stated where it belongs: in `persistence-stoolap-read-side`'s "A Real Profile::Production
Composition Exercises The Gate, With A Negative Control".

## Data Flow

```
   read-side session ──▶ Arc<dyn OffsetStore | DedupStore | ReadSideClaimStore>
                              │  (owned params built here, before the boundary)
                              ▼
                        run_blocking ──▶ spawn_blocking ──▶ Database (cloned handle)
                              │                                   │
                              │                    one process-global engine per DSN
                              │                    projection_offsets / _dedup / _claims
                              ▼                          (sync=full WAL, one file)
                     Ok / Ok(None) / Transient|Fatal|StaleOwner
                              ▲
                        clock.now()  (claim store only)
```

### Sequence: two tasks race `try_claim` on one fresh `claim_id`

```
 Task A                Task B              Stoolap engine (one row, MVCC)
   │ INSERT…DO NOTHING ─────────────────────────▶ row created, token=1
   │◀── affected = 1                             │
   │                    │ INSERT…DO NOTHING ────▶ conflict, no violation raised
   │                    │◀── affected = 0        │
   │                    │ SELECT ───────────────▶ owner=A, token=1, lease_until=L
   │                    │◀── row                 │
   │                    │ now < L  ⇒  Ok(None)   │        ← refusal, not an error
   │◀ Ok(Some(fence{1}))│                        │
```

```
 …later, lease lapsed; A and B both attempt takeover from token=5

 A: SELECT → (5, lapsed)        B: SELECT → (5, lapsed)
 A: UPDATE … WHERE fencing_token=5 AND lease_until<=now   ⇒ affected=1, token→6
 B: UPDATE … WHERE fencing_token=5 AND lease_until<=now   ⇒ affected=0  ⇒ Ok(None)
                                    …or a write conflict ⇒ ClaimError::Transient (retry-safe)
 Never: two Ok(Some) for one live lease, and never a token that is not strictly greater.
```

## File Changes

| File | Action | Description |
|---|---|---|
| `crates/persistence-stoolap/Cargo.toml` | Modify | `read-side` feature over four already-declared optional deps; `[[test]] name = "read_side_stores"` with `required-features = ["read-side"]` |
| `crates/persistence-stoolap/src/lib.rs` | Modify | `#[cfg(feature = "read-side")] pub mod read_side;` + three crate-root `pub use`s |
| `crates/persistence-stoolap/src/read_side/mod.rs` | Create | Module declarations; the same-process-only concurrency scope note |
| `crates/persistence-stoolap/src/read_side/offset.rs` | Create | `StoolapOffsetStore` (AD-4) + colocated unit tests |
| `crates/persistence-stoolap/src/read_side/dedup.rs` | Create | `StoolapDedupStore` (AD-4) + colocated unit tests |
| `crates/persistence-stoolap/src/read_side/claim.rs` | Create | `StoolapReadSideClaimStore` (AD-5/AD-6), `to_claim_error` shim + colocated unit tests |
| `crates/persistence-stoolap/src/persistence/stoolap_common.rs` | Modify | Host `token_for_storage`/`token_from_storage` as `pub(crate)` (AD-11) |
| `crates/persistence-stoolap/src/operation/reservation.rs` | Modify | Two functions removed, one import line added (AD-11). No behaviour change |
| `crates/persistence-stoolap/tests/read_side_stores.rs` | Create | Isolation, idempotence, takeover/fencing, concurrency race, reopen durability |
| `crates/service-sdk/Cargo.toml` | Modify | Dev-dependency feature list gains `"read-side"` (AD-13) |
| `crates/service-sdk/tests/read_side_progress_composition.rs` | Modify | Real Production composition + volatile negative control (AD-13) |
| `crates/persistence-api/src/read_side/**` | Unchanged | Contracts consumed as-is |
| `crates/persistence/src/postgres/**`, `crates/service-sdk/src/runtime/builder.rs` | Unchanged | No gate change, no Postgres change |

## Interfaces / Contracts

```rust
// crates/persistence-stoolap/src/read_side/{offset,dedup,claim}.rs
impl StoolapOffsetStore {
    pub async fn open(path: &Path) -> Result<Self, OffsetStoreError>;
}
impl StoolapDedupStore {
    pub async fn open(path: &Path) -> Result<Self, DedupStoreError>;
}
impl StoolapReadSideClaimStore {
    /// `clock` supplies "now" for expiry comparisons only; every `lease_until`
    /// is the caller's (AD-7).
    pub async fn open(path: &Path, clock: Arc<dyn ego_domain::Clock>) -> Result<Self, ClaimError>;
}

// All three, identically (AD-9): truthful by construction, never a hardcoded `true`.
fn is_durable(&self) -> bool { dsn_declares_sync_full(self.db.dsn()) }
```

```toml
# crates/persistence-stoolap/Cargo.toml — every dep already declared (AD-1)
read-side = ["dep:tokio", "dep:async-trait", "dep:chrono", "dep:ego-domain"]

[[test]]
name = "read_side_stores"
required-features = ["read-side"]
```

```rust
// crates/service-sdk/tests/read_side_progress_composition.rs (AD-13), positive case
let app = App::builder()
    .idempotency_enforcement_mode(IdempotencyEnforcementMode::Compatibility)
    .profile(Profile::Production)
    .read_side_progress("users-by-tenant", Arc::new(offset), Arc::new(dedup))
    .read_side_claims(Arc::new(claim))
    .build();                 // Ok — the unmodified gate accepts three durable Stoolap stores
// Negative control: swap exactly one store for the file's existing VolatileOffsetStore
// ⇒ CompositionError::Validation(RuntimeError::PersistenceNotConfigured(_))
```

## Concurrency Scope — what is claimed and what is not

| Scenario | Supported | Basis |
|---|---|---|
| One process, many async tasks, one store instance | **Yes** | Every mutation is one conditional statement under Stoolap MVCC (AD-4/AD-5); the loser is `Ok(None)` or a retry-safe `Transient` (AD-8) |
| One process, three store instances at one path (the production composition) | **Yes** | Stoolap's process-global registry shares one live engine per DSN while a handle is alive; proven for this crate by `tests/reservation_conformance.rs:258-311`. Re-proved here rather than assumed |
| Multiple OS processes over one file | **Not supported, not tested, not claimed** | Nothing in-tree establishes cross-process locking |
| Multi-node / distributed / leader election | **Not supported, not claimed** | Same honesty precedent as `StoolapEffectStore`'s `multi_node_safe: false` |

Each store's module doc states this scope. Fencing makes the *claim outcome* authoritative; it
does not cancel work a displaced owner already started — the same limit both existing leased-port
implementations document.

## Testing Strategy

| Layer | What to test | Approach |
|---|---|---|
| Unit — offset | Absent key reads `None`; a write to one `(projection_id, tag, tenant)` leaves every other key untouched; a repeat write overwrites with no conflict signal; the AD-4 fall-through path is reached on the first write | Colocated `#[cfg(test)]`, `tempfile` per test + `stoolap::test_failpoints::FailpointGuard` (the process-wide failpoint guard every DB test in this crate takes, `reservation.rs:589-591`) |
| Unit — dedup | Unseen reads `false`; `mark_seen` then `seen` is `true`; a repeat `mark_seen` succeeds and stays `true`; the same `event_id` under a different `(projection_id, tag)` is independent | Colocated; same guard |
| Unit — claim | Fresh grant; refusal while live; takeover after lapse mints a strictly greater token; `renew`/`release` reject a stale fence and a lapsed fence with `StaleOwner` and mutate nothing; after `release` the **row still exists** with an expired lease and is immediately reclaimable; `FencingToken::next() == None` surfaces `FencingExhausted`, never a wrapped token | Colocated, `TestClock` advanced explicitly (never a real sleep) |
| Unit — all three | `open()` refuses a path already held by a non-`sync=full` engine; `is_durable()` is `true` after a normal `open()` | Mirrors `reservation.rs:640-652` |
| Integration | **Concurrency**: several tasks in one process race `try_claim` on one fresh `claim_id`; exactly one `Ok(Some)`, every other `Ok(None)` or `Transient` — never a second grant, never `StaleOwner` | `tests/read_side_stores.rs`, `#[tokio::test(flavor = "multi_thread")]` + `tokio::spawn`, the shape `reservation_conformance.rs:192-245` already proves |
| Integration | **Shared engine**: three stores opened at one path observe one database | Explicit, per the concurrency table |
| Integration | **Restart**: written state survives a clean close and reopen of the same file, for each of the three stores | See the restart contract below |
| Composition | Real `Profile::Production` build over three real Stoolap stores succeeds; the same composition with exactly one volatile store is refused by the unmodified gate | `crates/service-sdk/tests/read_side_progress_composition.rs` (AD-13) |

**Restart contract — exactly what each reopen test proves, and what it does not.**

The shape is identical for all three: `tempfile::tempdir()` → `open()` → write state → **drop
every store handle for that path** → `open()` the same path again → assert. Dropping *every*
handle is load-bearing, not incidental: Stoolap keeps one process-global engine alive per DSN
while any handle exists, so a surviving handle would make the test prove nothing
(`reservation_conformance.rs:129-132` states the same caveat). Each test therefore uses a
dedicated tempdir with only its own store open.

| Store | What the reopen proves | What it explicitly does **not** prove |
|---|---|---|
| Offset | `read_offset` for the written key returns the identical `Offset`, and a never-written sibling key still returns `None` — so the reopened state is the persisted rows, not a rebuilt empty table | Nothing about crash, `kill -9`, or power loss |
| Dedup | `seen()` for the marked triple is still `true`, and an unmarked triple is still `false` | As above |
| Claim | A *different* owner's `try_claim` on the same `claim_id` still returns `Ok(None)` (the unexpired lease survived), the held fence still verifies through `renew`, and a fence released before the drop reopens as immediately reclaimable with a strictly greater token | As above. Also not multi-process reopen: the same OS process reopens the file |

TDD is on (`openspec/config.yaml`): each row lands RED first. `cargo test --workspace` does not
enable `read-side`, and CI's `--all-targets` must still compile without it — hence the
`required-features` entry (the `Cargo.toml:67-74` precedent). The `read-side` rows must also be run
with `--features read-side`.

## Threat Matrix

N/A — no routing, shell, subprocess, VCS/PR automation, executable-file classification, or
process-integration boundary. The one adjacent concern is caller-controlled text reaching SQL
(`projection_id`, `tag`, `tenant`, `event_id`, `owner_id` all originate outside the store); it is
handled by the same rule every store here follows: every value is bound as `$N`, never
interpolated into the statement.

## Migration / Rollout

No data migration. Each `open()` issues `CREATE TABLE IF NOT EXISTS`, so a fresh database and an
existing S1/S2/S3 database at the same path both work. The whole change is additive and gated: with
`read-side` off, `cargo build` and `cargo test --workspace` behave exactly as today. AD-11's move
inside `stoolap_common` is the only edit to shipped code and is behaviour-preserving.

**PR slicing — the proposal's hypothesis, confirmed with one correction.** The proposal guessed
PR1 = offset, PR2 = dedup, PR3 = claim, PR4 = composition. The design work confirms the first two
and the last, and splits the third:

| PR | Content | Est. lines | Why this cut |
|---|---|---|---|
| 1 | Cargo feature + `read_side/mod.rs` + `lib.rs` wiring + `StoolapOffsetStore` + `tests/read_side_stores.rs` (offset section) | ~280 | Carries the one-time scaffolding. Autonomous: an offset store that passes isolation, LWW and reopen tests is complete and revertible on its own |
| 2 | `StoolapDedupStore` + its tests | ~210 | Shares nothing with offset but the feature gate and the `run_blocking` shape (~30 lines of scaffolding, already landed in PR1). Merging 1+2 would total ~490 — over the 400-line budget to save 30 lines of duplication. Not worth it |
| 3 | AD-11 hoist + `StoolapReadSideClaimStore` + colocated unit tests | ~380 | The CAS, the fence-verified mutator and the token guards are one reviewable idea |
| 4 | Claim concurrency race + reopen-durability + shared-engine integration tests | ~250 | Combined with PR3 this is ~630. Split because the concurrency proof is the highest-value review target in the change and deserves its own diff, not the tail of a 630-line one |
| 5 | Composition test + negative control + the service-sdk dev-dep feature word | ~120 | Needs all three stores; last by construction |

`400-line budget risk: Medium` at this cut — every slice is forecast under 400 with margin. If PR3
measures under 400 once written, 3 and 4 may merge; `sdd-tasks` should measure rather than assume.
Feature-branch chain: PR1 targets the tracker branch, each later PR targets its predecessor.

## Open Questions

All three of the proposal's open questions are resolved above (AD-12, AD-13, AD-14). Two
implementation-level unknowns remain, both resolvable by experiment inside PR1 with no contract
consequence:

- [ ] Does Stoolap 0.4 return `affected` from a bare `UPDATE … WHERE <no match>` as `0` rather
      than an error? Every in-tree `UPDATE` reads `affected` this way
      (`reservation.rs:412`, `effect-store/src/stoolap/mod.rs:590`), so the shape is established —
      confirm it for AD-4 step 1 before relying on the fall-through, since a non-zero-on-no-match
      result would make `write_offset` skip its insert.
- [ ] Does an `INTEGER` column round-trip an `i64` `offset_value` exactly? `fencing_token` already
      does (`reservation.rs:59,209`), so this is a confirmation, not a risk.

## Risks Introduced By This Design

| Risk | Likelihood | Mitigation |
|---|---|---|
| `sdd-verify` reads the spec's "never a clock read performed inside the store" literally and fails AD-7 | Med | AD-7 states the reconciliation explicitly and names both existing implementations of the same port family that read an injected clock. If verify insists on the literal reading, the spec sentence — not the implementation — is what must change, because the trait signature carries no `now` parameter and is out of scope |
| The claim store's `Transient` classification depends on `is_write_conflict`'s pinned message-text arm (`"uncommitted changes from transaction"`) | Low | Pre-existing and already documented as brittle (`stoolap_common.rs:58-62`); this change adds a consumer, not the fragility. The concurrency test fails loudly if Stoolap changes the text |
| `ON CONFLICT … DO UPDATE` turns out to work on Stoolap 0.4 after all, making AD-4's three-step write look over-built | Low | Accepted. The cost is two extra statements on the first write only; the alternative is shipping an unproven statement shape on the durability path |
| A shared `assert_claim_store_conformance` is never written and the two claim backends drift | Med | Named as an explicit follow-up in AD-12 with its trigger, not left implicit |
