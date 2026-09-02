# `ego-integration-tests`

Invariants that a real PostgreSQL, real migrations, real transactions and real
concurrency can demonstrate — and that nothing else can.

An **independent Cargo workspace**, deliberately not a member of the root. The
root keeps building and testing with no Docker:

```bash
cargo test --workspace                                   # root; hermetic, never touches this
cargo run --manifest-path integration-tests/Cargo.toml \
    --bin run-suite                                      # this suite, explicit and opt-in
```

**The suite is started by its runner, not by `cargo test`.** The runner owns the
run's PostgreSQL: it starts one container, creates and migrates the template
database, runs the single test target, and destroys the container while its own
Tokio runtime is still alive — then exits with exactly the suite's code.

That last step is why the runner exists. A test binary has no suite-level
teardown, so a container held in a process-wide cell has its async `Drop` run at
process exit with no runtime left to drive it: three consecutive runs left three
containers behind. `cargo test` on this workspace still works as a way to reach
the target, but without the runner there is no PostgreSQL, and the suite says so
with the command that would have worked.

## Admission rules

This suite is small on purpose, and staying small is a requirement rather than a
preference. Every test here costs container startup, migration time and a slower
feedback loop for everyone; a suite that grows by accretion stops being run, and
a suite nobody runs proves nothing.

A scenario is admitted **only** if all of these hold.

1. **It traverses a capability end to end.** Not a function, not a variant, not
   a component in isolation.
2. **It could fail in a way no in-process test can detect.** The failure must
   depend on real PostgreSQL, real migrations, real transactions or real
   concurrency. If a scripted store could produce the same evidence, the
   scenario belongs in the fast suite.
3. **It duplicates nothing.** Anything already proven by unit, contract,
   conformance, `trybuild` or scripted-store tests must not be re-asserted here.
4. **It declares its own justification.** Every test states, in its own doc
   comment: the cross-cutting guarantee it demonstrates, the layers it
   traverses, and why in-process cannot show it. No justification, no test.

The rule those four serve: **infrastructure exists here only for what
infrastructure alone can show.**

### What explicitly stays in the fast suite

Named because these are the things most likely to be dragged in by habit: the
six HTTP refusal translations, response encoding and decoding, the operation-key
extractor, builder validation, and each individual branch of the reservation
store. All of them are already covered in-process, and all of them would run
identically against a container — which is exactly the definition of a test that
does not belong here.

## Budget

**PROD-012 gets at most four end-to-end tests.** A fifth is admitted only if a
new *infrastructure* risk justifies it — never a logical variant. "There is a
case we have not covered" is not a reason; that is what the fast suite is for.

That exception has been used more than once, and each use is classified separately
rather than counted against the end-to-end budget:

```
End-to-end scenarios ................. 4 / 4   (budget spent)
Durability precondition .............. 2
Real-process-death recovery .......... 2
PostgreSQL concurrency invariants .... 4
Durable-adapter conformance .......... 1
Migration transactional behaviour .... 1
Version-floor compatibility .......... 1
SQL-expression invariants ............ 1
Receipt identity isolation ........... 1
Schema/catalog assertions ............ 1
PROD-002 backend conformance ......... 1
PROD-002 backend-specific invariants . 1
PROD-002 provider composition ........ 1
Total infrastructure tests ........... 21
```

**This block was wrong, and the correction is the point of keeping this note.** It
read `Total infrastructure tests ... 6` while ten existed. Four tests had been
admitted with no row, no category and no recorded justification, and nothing
noticed, because nothing compared the ledger to the tree. That is a worse failure
than the one the note below already warns about: a ledger running *ahead* of the
tree retires an open question, but a ledger running *behind* it silently
understates what the suite costs and hides tests from the admission rules
altogether.

The counts describe the tests that exist, not the ones planned. An earlier version
of this block read `4 / 4` while scenario 4 was still unwritten — a ledger that runs
ahead of the tree is worse than none, because it retires the question it exists to
keep open. It is accurate now because the fourth scenario landed, not because the
number was aspirational.

**Drift is now a test failure, not a review question.** `tests/ledger.rs` requires
the directory, the module registration in `tests/infrastructure.rs` and this
document to describe the same set of tests, in every direction: a file added,
deleted or renamed without a matching row fails, a row with no file behind it
fails, and a file that exists but is never registered as a module — compiled
nowhere, run never — fails too. It is hermetic, starts no container, and the runner
executes it before provisioning PostgreSQL, so drift is caught in milliseconds. It
does **not** check that a justification is *true*; that stays a review question.

**A row means a row.** The guard counts a test as accounted for only from a Status
cell — a line that is a table row, with the path written as a code span. It does
not scan the document. That distinction is load-bearing rather than fussy: this
README cites test paths in prose as a matter of style, and
`concurrent_replicas_postgres.rs` is named both in its Status cell and in the
paragraph recording that the scenario is now guarded. An earlier version of the
guard scanned the whole file, so deleting that row — the only place its
justification lives — left the prose mention behind and the guard stayed green
while claiming every test had a row. Measured before and after: the same deletion
now fails, naming the test.

**Where the runner stops, and why it says which.** A preflight that could not run
is not a ledger that disagrees, and reporting the first as the second would be a
guard describing a case it does not cover. The runner builds the guard with
`--no-run` first: a failure there reports that the ledger was **never checked**,
naming the build. Only once it builds does a failing run report a real divergence.
Neither path provisions anything.

**Measured, six mutations, each restored byte-identically and confirmed by
SHA-256.** A guard is worth what its failures are worth, and each drift direction
had to kill it by a distinct assertion rather than by the same one twice.

| Mutation | Exit | Failed | Assertion that fired |
|---|---|---|---|
| A test file added, unregistered and undocumented | 101 | 2 | registration: present on disk, missing from `tests/infrastructure.rs`; ledger: present on disk, missing from `README.md` — both naming the new file |
| A test file renamed | 101 | 2 | the same two, naming the **new** name; the old name's stale entries are reported once the first assertion is satisfied |
| A test file deleted, its module and row left behind | 101 | 2 | the two *inverse* assertions: a module registered with no file, and a ledger row for a test that no longer exists |
| **A Status row deleted, its prose mention kept** | 101 | 1 | ledger: a test with no row, naming `concurrent_replicas_postgres`. **Was a false green** before the parse was anchored to table rows |
| The ledger stops citing tests by path | 101 | 2 | non-vacuity: zero rows parsed, so the set comparison would otherwise have been three-way-empty and green |
| The guard target made not to compile | 1 | — | the runner reports the ledger was **never checked**, explicitly not a divergence, and provisions nothing |
| No mutation | 0 | 0 | none — **negative control**, and it runs in 0.00s with no container |

The third row is the one that matters most, because it is the only drift where
the suite still *looks* complete: the module list and the ledger both still name a
test that is gone. The fourth is the guard turned on itself — every assertion here
compares two sets, and three empty sets are equal, so a parser that silently
matched nothing would pass every difference check while proving nothing.

**One coverage limit, stated rather than discovered.** Within each test the
assertions run in order, so a mutation that trips the first one leaves the second
unevaluated — the rename mutation reports the new name's absence before it reports
the old name's staleness. Both directions are covered across the matrix, never
both by a single run.

**The end-to-end budget is spent, and stays spent.** Every further *scenario* is a
variant, and variants belong in the fast suite. A further infrastructure test needs
its own new infrastructure risk, stated and measured the way each one below was —
and it lands in its own category rather than as a fifth end-to-end row.

**Categories are not a loophole.** Each one below exists because a test could not
honestly be described as an end-to-end scenario, not so that the four-scenario
budget could be worked around. A test whose risk is already covered by an existing
category and an existing test is a variant, whatever it is filed under.

The suite as a whole has a wall-clock budget, from issue #275: **≤5 minutes
total, ≤1–2 minutes for any individual slice.** A run that exceeds it is not
finished, even if every invariant is covered. Compilation and execution are
reported separately — a suite that takes twenty minutes to compile and ninety
seconds to run has not broken the budget, but it has found the next thing worth
fixing.

## The four PROD-012 scenarios

Each one exercises the whole protocol; together they cover it. None of them is a
variant of another.

| # | Scenario | Guarantee it demonstrates | Why in-process cannot show it | Status |
|---|---|---|---|---|
| 1 | Two identical `POST /register` | One execution, a durably completed response, and a replay served from PostgreSQL | The stored response has to survive a real commit and be read back through a real query — a scripted store returns whatever it was handed | `tests/infrastructure/replay_from_postgres.rs` |
| 2 | Same key, different payload | Permanent conflict, with no second execution, and the collided-with answer left intact | Reaching the fingerprint comparison at all depends on `(tenant_id, operation_key)` being genuinely unique. Without it the insert succeeds, a second row appears, and the conflict is never detected. The scenario runs under one tenant, so it loads the **tenant-scoped** partial index specifically | `tests/infrastructure/conflict_from_postgres.rs` |
| 3 | Recovery after an expired lease | Takeover under real fencing, without repeating steps already confirmed | Lease expiry is a clock-versus-row-state race resolved by the database; the receipt that stops the repeat was committed by a previous transaction | `tests/infrastructure/takeover_fencing_postgres.rs` |
| 4 | Two concurrent replicas | Exactly one obtains the permit; the other is refused without executing | Two concurrently released reservation attempts resolving to one durable winner is a database outcome; two runtimes sharing no memory can only be coordinated by the row. The test does not claim to observe SQL-level overlap of the two inserts — see its own docs for that boundary | `tests/infrastructure/concurrent_replicas_postgres.rs` |

**End-to-end consumed: 4 of 4.** The Status column is the ledger — it lives here
rather than in pull-request descriptions, which get buried. A row that gains a file
spends one of the four.

Row 4's original justification said "mutual exclusion between processes … a
single-process test cannot contend for it". That was overstated in two ways, and the
row above is the corrected version. Two runtimes in one OS process that share no
memory are coordinated solely by the row, which is the property that matters; and
the `lease_until <= $N` guard is **not** what this scenario loads — traced: the
loser's read finds `state = 'in_progress'` with `now` still before `lease_until` and
returns `OtherInProgress` without ever evaluating the takeover `UPDATE`. That guard
is the concurrency invariant's subject, not this row's.

## The PostgreSQL concurrency invariants

These are deliberately **store-level** rather than end-to-end, and that is not a
compromise: in each case the evidence *is* precise control of a transaction
holding a real lock, which HTTP cannot express. Dressing any of them up as an
end-to-end test would mean giving up the only mechanism that makes it a test.

| Invariant | Guarantee it demonstrates | Why it cannot be end-to-end | Status |
|---|---|---|---|
| Purge behind a locked row | A worker whose batch could be filled from unlocked eligible rows fills it, instead of waiting behind rows another worker holds | Head-of-line blocking is visible only with a real transaction holding real row locks. Measured: without `SKIP LOCKED` the statement waits on a locked tuple while free eligible rows sit untouched | `tests/infrastructure/purge_progress_postgres.rs` |
| Takeover blocked on a row lock | The takeover `UPDATE` re-checks `lease_until <= now` against the row it finally locks, not the row it read, so a lease renewed during the wait is not stolen | The window lives between two statements inside one `reserve()`. Forcing it open needs `SELECT … FOR UPDATE` held from outside while the contender blocks — not expressible over HTTP, and faking it would discard the only mechanism that makes it a test | `tests/infrastructure/fencing_window_postgres.rs` |
| Six contenders racing one expired lease (PROD-015 T-02.1, IS-3) | Six real contenders, all queued behind one row lock and released together, leave exactly one `TakenOver` winner; the fencing token advances by exactly one, never by the contender count | Requires forcing six real `UPDATE`s to genuinely block on one real row lock, with a deterministic poll (`wait_until_blocked`, AD-3) proving all six read the expired lease before any of them wrote — a scripted store has no row lock to serialize contenders on | `tests/infrastructure/lease_contention_postgres.rs` |
| N-way append race + NULL-tenant identity behavior (PROD-015 T-03.1/T-03.2, IS-2 post-`23505` scope + IS-5) | Four real racers appending to one fresh stream leave exactly one winner, and each loser's `Conflict` reports the real winning version obtained only after its own transaction aborted on `ux_events_identity_tenant` and re-read the stream on a different connection; separately, `Option::None` tenant identity is genuinely unique under `ux_events_identity_systemwide` (not exempt), does not collide with a concrete tenant's identical `(aggregate_type, aggregate_id, version)`, and two distinct systemwide streams never falsely collide or merge | Requires forcing four real transactions to collide on a real unique index past a real abort, then re-reading on a genuinely separate connection — a scripted store has no unique-constraint abort to re-read past. The pre-check clause of SC-2 is out of scope by design: IS-1's conformance run already exercises it | `tests/infrastructure/events_identity_race_postgres.rs` |

Why the takeover-window invariant was admitted, with the evidence: `reservation.rs` stated in its own comment
that this predicate was "currently unguarded by any test here", and this document's
own rebuild note called it the highest-value missing guarantee, recording that
neutralising the predicate left the conformance suite green. Re-measured on the
rebuild — neutralising it now fails this test while `replay_from_postgres`,
`conflict_from_postgres` and `takeover_fencing_postgres` all stay green. Named
rather than counted, so the claim does not silently go stale when scenario 4 lands.

Row 2's justification took two corrections, both found by checking it rather than
trusting it:

1. It claimed the fingerprint comparison was "a real uniqueness constraint under a
   real transaction". It is an `if` in Rust over a value read back from the row.
   The real database-only mechanism is one step earlier — reaching that comparison
   at all requires genuine uniqueness on `(tenant_id, operation_key)`.
2. It then claimed the test demonstrated *both* complementary partial indexes. It
   does not: the scenario runs under one tenant, and deleting only the systemwide
   (`WHERE tenant_id IS NULL`) index leaves it green. Measured. The claim is now
   scoped to the tenant-scoped index, which is the one it actually loads.

A justification nobody checks is how a scenario earns a slot it does not deserve,
and a justification that is *nearly* right is the harder case — it survives
review. Both of these did, once. Worth re-reading this column before the last
slot is spent.

Scenario 4 is the one issue #275 called the highest-value invariant in the
backlog. **It is now guarded** by `tests/infrastructure/concurrent_replicas_postgres.rs`;
this paragraph previously ended "today it is guarded by nothing", which stopped
being true when that row gained a file and was never updated.

An audit found that row's aggregate writes were not real: both racing
replicas ran `RegisterUser` over an in-memory `EntityRuntimeBuilder::new().build()`
that the two replicas did not even share, so only the reservation-ownership
race was ever durable — "the winner executed" and "the winner's write
committed" were indistinguishable. Both replicas now write through the same
`EntityEventStores::open` + `compose_entity_runtimes` composition production
uses, and the test asserts the real `events`/`operation_receipts` rows
directly: exactly one of each for the contended key, after the race.

## Recovery and durability

Neither of these is an end-to-end *scenario* in the sense the four above are, and
neither is a variant of one. They are filed here because each rests on an
infrastructure risk none of the four carries.

| Test | Guarantee it demonstrates | Why in-process cannot show it | Status |
|---|---|---|---|
| Recovery after a real process death | After a process dies between the two halves of one dual-aggregate operation, a retry resumes rather than repeats: the confirmed half is not re-executed, and the missing half runs exactly once | The evidence **is** a real crash. A child process is killed by `SIGABRT` between the two aggregates, so the partial durable state is produced by an execution that genuinely stopped rather than by a fixture arranging one. No in-process test can leave a half-finished operation behind, because unwinding is not dying. Unix-only, structurally: reading a signal from an exit status is `std::os::unix`, and degrading it to "any non-zero exit" would admit a panic or a missing database as a crash | `tests/infrastructure/dual_aggregate_crash_recovery_postgres.rs` |
| Recovery after a real process death, single-aggregate case | After a process dies once a single-aggregate operation's event, receipt and reservation have all already committed, a retry is answered by the reservation's own stored response — not by re-running the handler or writing a second event or receipt | Same reasoning as the dual-aggregate row, for the complementary shape: that scenario proves resumption of an *unfinished* operation; this one proves a *finished* one is never repeated after a real crash, which needs its own child process and its own `SIGABRT` — nothing here is inferred from the dual-aggregate case. Deliberately narrower than an HTTP round trip: it drives `EnsureOrg`, a minimal one-aggregate `#[idempotent]` operation over the same `TenantOrganization` domain type, directly through `Runtime::resolve`, because the property under test — durable reservation replay under a real crash — lives below the transport | `tests/infrastructure/single_aggregate_crash_recovery_postgres.rs` |
| An entity's events and receipt outlive their runtime | What the composition root actually wires is durable — the events *and* the confirmed receipt are read back by a second runtime | This is the precondition the crash scenario rests on, and it was false when written: **no `EntityRuntime` anywhere was given a durable event store**, so production took `EntityRuntimeBuilder`'s in-memory default. Receipts live in the event store, so a crash destroyed the events and the receipt together. A recovery test over that would have failed for the wrong reason, and a passing one would have proved only that a fixture kept its own map. Nothing in-process can distinguish the two, because in-process is exactly the condition being ruled out | `tests/infrastructure/durable_entity_progress_postgres.rs` |
| `EntityEventStores::open` declares `Profile::Production` and its snapshot stores are durable | `open(pool)` is the only thing that yields `Profile::Production` (PROD-013 AD-8), and the snapshot store it hands both aggregates is a real `PostgreSQLSnapshotStore` over that pool, not the in-memory default it replaced (AD-9) | The precondition was false here too: `open()` wired durable events but left both aggregates' snapshots in process memory, silently. A written snapshot must survive a fresh `open()` against the same pool after the writing instance is dropped — an in-memory store cast to the same `Arc<Mutex<dyn Snapshot + Send>>` field type would pass a type check and fail exactly this | `tests/infrastructure/entity_event_stores_wiring_postgres.rs` |

The durability test earns its own slot rather than being folded into the crash
scenario: if the two were one test, a regression that put the entity runtime back
on its in-memory default would surface as a confusing failure inside a crash
scenario instead of as the plain statement that nothing durable was wired.

## Durable-adapter conformance

`PostgreSQLEventStore` and `PostgresOperationReservationStore` judged against
the identical shared conformance harnesses (`ego_testkit::assert_event_store_conformance`,
`ego_testkit::assert_reservation_store_conformance`) the in-memory adapters
satisfy — the same definitions, never a re-derived or weakened set. IS-6
(a staged, uncommitted append is invisible to a reader on a distinct pooled
connection, and a unit of work dropped without commit persists nothing) is
retired into this same run per D-4/AD-4: demonstrated here, with no separate
test or row of its own.

| Test | Guarantee it demonstrates | Why in-process cannot show it | Status |
|---|---|---|---|
| Event store and reservation store, against the shared conformance harnesses | `PostgreSQLEventStore` and `PostgresOperationReservationStore` satisfy the same `EventStore<E>`/`OperationReservationStore` conformance definitions the in-memory adapters satisfy, including that an uncommitted staged append is invisible to a reader on a distinct connection and a dropped unit of work persists nothing (IS-6) | No in-memory double has a real transaction, a real second pooled connection, or real `READ COMMITTED` cross-connection visibility — a staging map cannot misrepresent isolation it never had to implement. The reservation store's fencing/CAS assertions likewise need a real row and a real conditional `UPDATE`, which a scripted store cannot misrepresent in the way this suite is built to catch | `tests/infrastructure/durable_store_conformance_postgres.rs` |

## Read-side durable progress (PROD-014B)

`PostgreSQLOffsetStore` and `PostgreSQLDedupStore` — the durable pair behind
the `OffsetStore`/`DedupStore` read-side SPI ports, closing PROD-014A's F-1
gap. Each case obtains its database via `isolated_database()`, and the
restart-survival case drops the store and its pool entirely before reading
back through a brand-new pool, so nothing here is satisfiable by in-process
state. See the file's own module doc for what this suite deliberately does
not claim about execution exclusion (PROD-014B AD-6) — that gap is named,
distinct follow-up **PROD-014C — Atomic Read-Side Event Claiming**.

| Test | Guarantee it demonstrates | Why in-process cannot show it | Status |
|---|---|---|---|
| Offset survives a process restart, tenant isolation, last-write-wins writes, dedup convergence (sequential and concurrent), tenant-independent dedup identity, both stores report durable, unapplied-migration classification | The durable pair behaves the way `spec.md` states against real PostgreSQL: an offset outlives the process that wrote it, a never-written tenant never leaks another tenant's row, a later write silently wins, two `mark_seen` calls on one identity converge to one row whether sequential or concurrent, dedup identity carries no tenant, both `is_durable()` report `true`, and a missing table classifies `Fatal`, never `Transient` | Restart survival and tenant isolation are properties of the stored rows, not of an in-memory struct — a scripted double has nothing to lose across a restart. Dedup convergence under real concurrency needs a real `ON CONFLICT … DO NOTHING` resolved by the database, not a mutex a test controls. The unapplied-migration case needs a real `42P01` from a real catalog lookup — no scripted store has a catalog to be missing from | `tests/infrastructure/read_side_progress_postgres.rs` |

## Migration transactional behaviour

The offline `backfill_aggregate_type` operator step
(`crates/persistence/src/postgres/aggregate_type_backfill.rs`) has two
distinct judgment stages — a preflight over values computed in memory, and a
post-verification over the rows as they were actually written — and each
stage's refusal is a genuinely different event from the other, not a
synonym: a preflight abort means nothing was ever written; a
post-verification rollback means rows *were* written and then discarded.
Four cases (PROD-015 T-04.1–T-04.4, IS-4) cover both stages plus the two
paths that only exist once the preflight and post-verification both pass —
committing over zero rows, and the exact reverse — each proved by a
byte-identical digest of the table before and after (AD-5).

| Invariant | Guarantee it demonstrates | Why it cannot be end-to-end | Status |
|---|---|---|---|
| C1 — abort before any write | A preflight refusal (`Aborted(NoRegisteredTypeMatches)`) leaves the table byte-identical, proving statement *ordering* — the transaction is dropped before any `UPDATE`, not rolled back | Requires a real migrated table and the real abort closure's statement ordering; no in-memory double has a transaction to drop mid-scan | `tests/infrastructure/aggregate_type_backfill_postgres.rs` |
| C2 — rollback after a completed write | A post-verification refusal (`RolledBack(StreamVersionsAreNotConsecutiveFromOne)`) leaves the table byte-identical and `aggregate_type` still nullable, proving the rollback — not merely ordering — is what discards writes that were genuinely made | Only a real transaction rollback against a real migrated table can demonstrate rows written and then discarded; an in-memory double has nothing to roll back | `tests/infrastructure/aggregate_type_backfill_postgres.rs` |
| C3 — zero-row commit | A run over zero eligible rows commits, including the schema-level `SET NOT NULL` — the last statement before commit — so "committed" cannot be confused with "committed nothing" | Requires a real migrated table and a real catalog read to distinguish "committed the intended statement" from "committed nothing at all" | `tests/infrastructure/aggregate_type_backfill_postgres.rs` |
| C4 — revert round trip | `revert_aggregate_type_column` rejoins exactly the state that preceded a successful backfill, and the column no longer exists afterward | Requires the real forward and reverse migration paths against a real, migrated database | `tests/infrastructure/aggregate_type_backfill_postgres.rs` |

## Version-floor compatibility

A second, separately-owned PostgreSQL 14 container (`pg14_database()`),
distinct from the main suite's PG16 container — never a second full run of
the main suite. Four narrow cases (PROD-015 T-05.1–T-05.4, IS-9) exercise
exactly the artifacts whose SQL syntax is version-sensitive: every
`NULLS NOT DISTINCT`-avoidance pattern in this schema (the paired partial
unique indexes on `events`, `operation_reservations`, `operation_receipts`
and `snapshots`) exists specifically because that syntax arrived in
PostgreSQL 15 and this workspace declares 14 as its floor.

**Explicitly not run on PG14** (`design.md` AD-6): IS-1, IS-2, IS-3, IS-6,
and IS-4's C1/C2 abort/rollback cases, plus all sixteen pre-existing tests.
This file targets exactly T0–T3, nothing else.

| Invariant | Guarantee it demonstrates | Why it cannot be end-to-end | Status |
|---|---|---|---|
| T0 — anti-vacuity guard | The PG14 container genuinely reports a PostgreSQL 14.x server version, so a container-tag typo cannot silently run this file against PG16 | Only the real target engine can report its own version; the same "three-empty-sets" discipline `tests/ledger.rs` already applies | `tests/infrastructure/pg14_compatibility.rs` |
| T1 — the full migration set applies cleanly on PG14 | Every version-sensitive schema artifact from migrations 008, 010, 011 and 012 genuinely exists after `migrations::run()` executes directly against PG14 (no template clone) | The migrations carry no tracking table; only a real catalog read against the real target version can confirm the idempotent SQL actually applied there, not merely that the fixture did not panic | `tests/infrastructure/pg14_compatibility.rs` |
| T2 — systemwide duplicate refused with `23505` | `ux_events_identity_systemwide`'s partial unique index refuses a duplicate `(aggregate_type, aggregate_id, version)` under `tenant_id IS NULL` on PG14, the same way it does on PG16 | `NULLS NOT DISTINCT` is unavailable on the declared 14 floor; only a real duplicate insert against the real partial index proves the two-index pattern is what actually holds there | `tests/infrastructure/pg14_compatibility.rs` |
| T3 — backfill/revert round trip | `backfill_aggregate_type` and `revert_aggregate_type_column` round-trip cleanly against a real PG14-migrated table, mirroring C4's proof on PG16 | Requires the real forward and reverse migration paths against a real, migrated PostgreSQL 14 database | `tests/infrastructure/pg14_compatibility.rs` |

## SQL-expression invariants

| Invariant | Guarantee it demonstrates | Why it cannot be end-to-end | Status |
|---|---|---|---|
| `oldest_completed` reports the earliest surviving completion | The retention backlog gauge reports the *earliest* `completed_at` still held, `Empty` when nothing is completed, and never `Unsupported` | The two store implementations answer by different means — `Iterator::min` over a map versus a SQL aggregate — so a `MIN` written as `MAX`, or a predicate that admits in-progress rows, is invisible to every test that does not execute the statement. The retention worker's own gauge tests drive the in-memory store, so they prove the worker reads and converts an answer, never that the durable answer is right. It is a one-token error that ships silently and reports a backlog age wrong in the reassuring direction | `tests/infrastructure/oldest_completed_postgres.rs` |

## Receipt identity isolation

The receipt identity is `(tenant_id, aggregate_type, aggregate_id,
operation_key)`. `schema_index_assertion.rs` pins the shape of the two
partial unique indexes that enforce it; it never files a row. This test files
two, holding three of the four fields fixed and varying exactly one, and
proves both directions: no collision across the varied field, and — within
one fixed scope — a genuine retry replays while a different request reusing
the identity is refused rather than overwriting what is stored.

| Invariant | Guarantee it demonstrates | Why it cannot be end-to-end | Status |
|---|---|---|---|
| Each identity field isolates independently | Two receipts agreeing on three of the four identity fields and differing on the fourth never collide and each keeps its own outcome; within one fixed scope a genuine retry replays and a fingerprint mismatch conflicts without disturbing the stored row | `conflict_from_postgres.rs` loads only the reservation table's `(tenant_id, operation_key)` pair — the reservations table has no `aggregate_type`/`aggregate_id` column at all. Only a real insert against the real receipt indexes can show that varying `aggregate_type` or `aggregate_id` alone does not collide; the catalog shape alone cannot rule out a narrower `ON CONFLICT` target or a dropped predicate | `tests/infrastructure/receipt_identity_isolation_postgres.rs` |

## Schema and catalog assertions

Not a scenario, and deliberately not named `*_postgres` like its neighbours: it
traverses no framework layer at all. It asserts the shape of the indexes every
other test in this suite depends on.

| Assertion | Guarantee it demonstrates | Why it cannot be in-process | Status |
|---|---|---|---|
| Partial-unique-index pairs, as the catalog reports them | Every table whose identity is unique per tenant *and* per systemwide stream carries both halves of its pair, with the exact columns, in the exact order, unique, over exactly complementary NULL predicates | It reads `pg_index`, `pg_class` and `pg_attribute` — what the server is actually enforcing. Matching the `.sql` files as text would only prove a file says what it says: a migration can be shadowed by an earlier one, an `IF NOT EXISTS` can silently decline to replace a differently-shaped index, and a column can be dropped and re-added under a name that still matches | `tests/infrastructure/schema_index_assertion.rs` |

This assertion is what lets the scenario rows above justify themselves by naming
an index. Row 2's justification depends on `(tenant_id, operation_key)` being
genuinely unique; without something reading the catalog, that dependency would
rest on a migration file nobody verified was the one in force.

## PROD-002 durable effect-store: PostgreSQL conformance and backend invariants

Two further infrastructure tests, filed under their own PROD-002 infrastructure
risk rather than as PROD-012 variants: `PostgresEffectStore`'s durability and
multi-node claim exclusivity are real-Postgres-only properties, in exactly the
sense the categories above already are for PROD-012's own store.

| Test | Guarantee it demonstrates | Why in-process cannot show it | Status |
|---|---|---|---|
| Tier 1/2/3 conformance against a real PostgreSQL | The shared `EffectStateStore`/`EffectDedupStore` port contract (Tier 1); state and dedup reservations survive a genuine close→reopen against the same tables (Tier 2); two independently-owned live claimers sharing the same tables never both hold an overlapping valid claim (Tier 3) | Tier 2/3 need a factory that opens more than one live store instance against the *same* backing storage — the property a restart or a second node relies on — which no in-process double can misrepresent, because an in-process double has no backing storage independent of the instance holding it | `tests/infrastructure/effect_store_postgres_conformance.rs` |
| `PostgresEffectStore`-specific claim/lease/retention behavior | Claim exclusivity (G1), expired-lease-scoped reclaim (AD-4), epoch-fenced writes, atomic dedup reservation (AD-8), the AD-9 retention batch bound, and the G10 clock-injection guarantee | Each is a property of what the real database enforces under a real conditional `UPDATE`, a real primary-key upsert, or real row atomicity — none of which a scripted double can misrepresent in a way this suite would catch | `tests/infrastructure/effect_store_postgres_unit.rs` |
| PROD-002 PR5 Phase 7.5: `RuntimeBuilder::with_effect_store` composed with a real `PostgresEffectStore` | Registering a real, networked `PostgresEffectStore` through the composition seam makes the runtime's `RuntimeEffectAcceptor`/delivery runner actually dispatch THROUGH it end to end — not merely that the seam type-checks. Deliberately not a re-test of the conformance rows above | The in-process sibling (`crates/service-sdk/tests/effect_store_composition.rs`) proves the same seam against a test double and a real embedded Stoolap store; neither exercises sqlx's real connection pool, schema creation and migration path, which only a real networked PostgreSQL can | `tests/infrastructure/effect_store_composition_postgres.rs` |

Relocated here (PROD-002 G11) from the old per-crate `crates/integration-tests`
— one `testcontainers` container per test file — onto this suite's shared
container and per-test isolated database, the same consolidation PROD-012's own
tests went through.

**No migration wiring was needed.** `PostgresEffectStore::connect` already
creates and migrates its own schema on every call (AD-10's hand-rolled runner,
every statement `CREATE ... IF NOT EXISTS`) — unlike `ego-persistence`'s
tables, which this suite's template pre-migrates once into `public`,
`effect_state`/`effect_dedup` live under a schema the store itself creates the
first time it connects. Each test's isolated database gets its own copy of
that schema, migrated by the store, not by the runner's template step.

## IS-8 mutation proofs: production code, not the ledger guard

The mutation table above (under "Budget") proves `tests/ledger.rs` itself
catches drift. This table is the separate IS-8 obligation: proof that the
*production* code path each PostgreSQL-only test claims to guard actually
fails that test when neutralised. Every row here follows the same recipe —
SHA-256 recorded before the edit, the edit applied, the suite run, the
failure recorded, `git checkout --` reverts the file, SHA-256 recorded again
and confirmed equal to the first, then the suite re-run once more as a
negative control. No production file is left modified by this change; every
mutation here is temporary and reverted by design (D-8, Rollback Plan).

| ID | Production file mutated | Change | Test that must fail, and how | SHA-256 (before = after-restore, confirmed equal) |
|---|---|---|---|---|
| M3 | `crates/persistence/src/postgres/event_store.rs` | `PostgresEventStoreUnitOfWork::append`'s event `INSERT` routed off the held transaction: `.execute(&mut *self.tx)` → `.execute(&self.pool)` | `durable_store_conformance_postgres::postgres_event_store_conformance` fails: "an uncommitted append must not be visible to a reader, saw 1 event(s)" (panic at `crates/testkit/src/event_store.rs:358`). Every other test, including `postgres_reservation_store_conformance`, stayed green — this is the only check on unit-of-work atomicity | `9dc88a462c3a16a582d078da321c6e651125497df88022f98910fddc0075e467` |
| M2 | `crates/persistence/src/postgres/aggregate_type_backfill.rs` | Neutralised the post-verification rollback for a discontinuous stream: in the `StreamVersionsAreNotConsecutiveFromOne` branch, `tx.rollback().await?` → `tx.commit().await?` | `aggregate_type_backfill_postgres::c2_a_rollback_after_a_completed_write_leaves_the_table_byte_identical` fails on the digest comparison: "a rollback after at least one completed UPDATE must leave the table byte-identical" (`left: "9e507614b4d844a157205009cac5915e", right: "9f606b0b22bdedd94ab5d35082324b53"`) — the rows written before the discontinuity check now persist. Every other test, including `c1`/`c3`/`c4` in the same file, stayed green: 51 passed, 1 failed, 1 ignored — SC-8's "pre-existing suite stays green" is satisfied exactly as written for this mutation | `3381e5cf4852c9d99e6a30f227c6d0c625efc6ef84e92abdf6ee32bbc7085fed` |
| M1a | `crates/persistence/src/postgres/reservation.rs` | Neutralised the takeover `UPDATE`'s lease-expiry predicate: the bind for `lease_until <= $7` changed from `.bind(now)` to `.bind(now + chrono::Duration::days(3650))`, so the predicate is satisfied by any `lease_until` a reservation could realistically carry | `fencing_window_postgres::a_takeover_waiting_on_the_row_lock_rechecks_the_lease_it_finds_not_the_one_it_read` fails: "the takeover must be refused … Got TakenOver(…)" — the renewed lease is wrongly taken over. Every other test, including `lease_contention_postgres` (the fencing-token CAS alone is still sufficient there, since the lease is genuinely expired), stayed green: 45 passed, 1 failed, 1 ignored | `e69b9bf6cefb6a51fd60ad1af9e75c1fd5fe05a1466f7d4be41425a8221118f3` |
| M1b | `crates/persistence/src/postgres/reservation.rs` | Neutralised the takeover `UPDATE`'s fencing-token compare-and-swap: `AND fencing_token = $6` → `AND $6 = $6` | The whole suite stays green: 46 passed, 0 failed, 1 ignored — re-measured, confirming the code comment's own claim that the lease-expiry predicate alone is sufficient and the token CAS is redundant given it | `e69b9bf6cefb6a51fd60ad1af9e75c1fd5fe05a1466f7d4be41425a8221118f3` |
| M1c | `crates/persistence/src/postgres/reservation.rs` | Both M1a and M1b applied simultaneously | `lease_contention_postgres::six_contenders_racing_one_expired_lease_leave_exactly_one_winner` fails, observing all six contenders report `TakenOver` (`left: 6, right: 1`) instead of exactly one; `fencing_window_postgres` also fails, for the same reason as M1a — both load the identical predicate pair. SC-8's literal "and the pre-existing suite stays green" does not hold here; the demonstrable, narrower claim is: the new test fails, and no test that does not exercise this predicate pair fails. 44 passed, 2 failed, 1 ignored | `e69b9bf6cefb6a51fd60ad1af9e75c1fd5fe05a1466f7d4be41425a8221118f3` |

Negative control after each revert: full real-container run, 0 failures.

## Conventions

- **One shared PostgreSQL per run**, isolated per test by schema or database.
  Never one container per test — per-test containers are what made the previous
  suite unusable.
- **Migrations run once per run**, not once per test.
- **No arbitrary sleeps.** Synchronise on a signal, or poll with an explicit
  deadline. A fixed timeout standing in for a condition is not acceptable.
- **Reuse `ego-testkit`'s conformance harnesses** against the durable
  implementations — the same definitions, never a parallel copy.

## Running it

The ledger guard needs no Docker and no database, so it can be run on its own:

```bash
cargo test --manifest-path integration-tests/Cargo.toml --test ledger
```

The full suite requires a reachable Docker daemon. The runner executes the ledger
guard first and stops there if it fails, so a drifted ledger costs milliseconds
rather than a container start:

```bash
colima start                                             # or Docker Desktop
export DOCKER_HOST="unix://$HOME/.colima/default/docker.sock"
cargo run --manifest-path integration-tests/Cargo.toml --bin run-suite
```

The runner reports its own timings, so the budget is observable rather than
claimed:

```text
[integration-tests] provisioned in 1.82s · template migrated at 2.28s · suite finished at 15.9s · reclaimed at 16.2s
```
