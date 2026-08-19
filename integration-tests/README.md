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
Durability precondition .............. 1
Real-process-death recovery .......... 1
PostgreSQL concurrency invariants .... 2
SQL-expression invariants ............ 1
Schema/catalog assertions ............ 1
Total infrastructure tests ........... 10
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

**Measured, four mutations, each restored byte-identically and confirmed by
SHA-256.** A guard is worth what its failures are worth, and each drift direction
had to kill it by a distinct assertion rather than by the same one twice.

| Mutation | Exit | Failed | Assertion that fired |
|---|---|---|---|
| A test file added, unregistered and undocumented | 101 | 2 | registration: present on disk, missing from `tests/infrastructure.rs`; ledger: present on disk, missing from `README.md` — both naming the new file |
| A test file renamed | 101 | 2 | the same two, naming the **new** name; the old name's stale entries are reported once the first assertion is satisfied |
| A test file deleted, its module and row left behind | 101 | 2 | the two *inverse* assertions: a module registered with no file, and a ledger row for a test that no longer exists |
| The ledger stops citing tests by path | 101 | 2 | non-vacuity: zero rows parsed, so the set comparison would otherwise have been three-way-empty and green |
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

Both are deliberately **store-level** rather than end-to-end, and that is not a
compromise: in each case the evidence *is* precise control of a transaction
holding a row lock, which HTTP cannot express. Dressing either up as an
end-to-end test would mean giving up the only mechanism that makes it a test.

| Invariant | Guarantee it demonstrates | Why it cannot be end-to-end | Status |
|---|---|---|---|
| Purge behind a locked row | A worker whose batch could be filled from unlocked eligible rows fills it, instead of waiting behind rows another worker holds | Head-of-line blocking is visible only with a real transaction holding real row locks. Measured: without `SKIP LOCKED` the statement waits on a locked tuple while free eligible rows sit untouched | `tests/infrastructure/purge_progress_postgres.rs` |
| Takeover blocked on a row lock | The takeover `UPDATE` re-checks `lease_until <= now` against the row it finally locks, not the row it read, so a lease renewed during the wait is not stolen | The window lives between two statements inside one `reserve()`. Forcing it open needs `SELECT … FOR UPDATE` held from outside while the contender blocks — not expressible over HTTP, and faking it would discard the only mechanism that makes it a test | `tests/infrastructure/fencing_window_postgres.rs` |

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

## Recovery and durability

Neither of these is an end-to-end *scenario* in the sense the four above are, and
neither is a variant of one. They are filed here because each rests on an
infrastructure risk none of the four carries.

| Test | Guarantee it demonstrates | Why in-process cannot show it | Status |
|---|---|---|---|
| Recovery after a real process death | After a process dies between the two halves of one dual-aggregate operation, a retry resumes rather than repeats: the confirmed half is not re-executed, and the missing half runs exactly once | The evidence **is** a real crash. A child process is killed by `SIGABRT` between the two aggregates, so the partial durable state is produced by an execution that genuinely stopped rather than by a fixture arranging one. No in-process test can leave a half-finished operation behind, because unwinding is not dying. Unix-only, structurally: reading a signal from an exit status is `std::os::unix`, and degrading it to "any non-zero exit" would admit a panic or a missing database as a crash | `tests/infrastructure/dual_aggregate_crash_recovery_postgres.rs` |
| An entity's events and receipt outlive their runtime | What the composition root actually wires is durable — the events *and* the confirmed receipt are read back by a second runtime | This is the precondition the crash scenario rests on, and it was false when written: **no `EntityRuntime` anywhere was given a durable event store**, so production took `EntityRuntimeBuilder`'s in-memory default. Receipts live in the event store, so a crash destroyed the events and the receipt together. A recovery test over that would have failed for the wrong reason, and a passing one would have proved only that a fixture kept its own map. Nothing in-process can distinguish the two, because in-process is exactly the condition being ruled out | `tests/infrastructure/durable_entity_progress_postgres.rs` |

The durability test earns its own slot rather than being folded into the crash
scenario: if the two were one test, a regression that put the entity runtime back
on its in-memory default would surface as a confusing failure inside a crash
scenario instead of as the plain statement that nothing durable was wired.

## SQL-expression invariants

| Invariant | Guarantee it demonstrates | Why it cannot be end-to-end | Status |
|---|---|---|---|
| `oldest_completed` reports the earliest surviving completion | The retention backlog gauge reports the *earliest* `completed_at` still held, `Empty` when nothing is completed, and never `Unsupported` | The two store implementations answer by different means — `Iterator::min` over a map versus a SQL aggregate — so a `MIN` written as `MAX`, or a predicate that admits in-progress rows, is invisible to every test that does not execute the statement. The retention worker's own gauge tests drive the in-memory store, so they prove the worker reads and converts an answer, never that the durable answer is right. It is a one-token error that ships silently and reports a backlog age wrong in the reassuring direction | `tests/infrastructure/oldest_completed_postgres.rs` |

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
