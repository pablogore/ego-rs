# `ego-integration-tests`

Invariants that a real PostgreSQL, real migrations, real transactions and real
concurrency can demonstrate — and that nothing else can.

An **independent Cargo workspace**, deliberately not a member of the root. The
root keeps building and testing with no Docker:

```bash
cargo test --workspace                                   # root; hermetic, never touches this
cargo test --manifest-path integration-tests/Cargo.toml  # this suite, explicit and opt-in
```

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

That exception has been used exactly once, and it is classified separately rather
than counted as a fifth end-to-end scenario:

```
End-to-end scenarios ................. 4 / 4
PostgreSQL concurrency invariants .... 1 / 1
Total infrastructure tests ........... 5
```

The counts describe the tests that exist, not the ones planned. An earlier version
of this block read `4 / 4` while scenario 4 was still unwritten — a ledger that runs
ahead of the tree is worse than none, because it retires the question it exists to
keep open. It is accurate now because the fourth scenario landed, not because the
number was aspirational.

**The end-to-end budget is spent.** Every further scenario is a variant, and
variants belong in the fast suite. A sixth infrastructure test needs its own new
infrastructure risk, stated and measured the way the concurrency invariant's was.

The concurrency invariant is `tests/fencing_window_postgres.rs`, and it is
deliberately **store-level** rather than end-to-end: the evidence it needs is
precise control of a transaction holding a row lock, which HTTP cannot express.
Its admission rests on the clause above and nothing looser — a distinct
infrastructure risk, covered by none of the four, which two independent sources
named as the highest-value guarantee with no test. See the entry below.

This does not open a door to growth by variant. A sixth test needs its own new
infrastructure risk, stated and measured the same way.

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
| 1 | Two identical `POST /register` | One execution, a durably completed response, and a replay served from PostgreSQL | The stored response has to survive a real commit and be read back through a real query — a scripted store returns whatever it was handed | `tests/replay_from_postgres.rs` |
| 2 | Same key, different payload | Permanent conflict, with no second execution, and the collided-with answer left intact | Reaching the fingerprint comparison at all depends on `(tenant_id, operation_key)` being genuinely unique. Without it the insert succeeds, a second row appears, and the conflict is never detected. The scenario runs under one tenant, so it loads the **tenant-scoped** partial index specifically | `tests/conflict_from_postgres.rs` |
| 3 | Recovery after an expired lease | Takeover under real fencing, without repeating steps already confirmed | Lease expiry is a clock-versus-row-state race resolved by the database; the receipt that stops the repeat was committed by a previous transaction | `tests/takeover_fencing_postgres.rs` |
| 4 | Two concurrent replicas | Exactly one obtains the permit; the other is refused without executing | Two concurrently released reservation attempts resolving to one durable winner is a database outcome; two runtimes sharing no memory can only be coordinated by the row. The test does not claim to observe SQL-level overlap of the two inserts — see its own docs for that boundary | `tests/concurrent_replicas_postgres.rs` |

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

## The PostgreSQL concurrency invariant

| Invariant | Guarantee it demonstrates | Why it cannot be end-to-end | Status |
|---|---|---|---|
| Takeover blocked on a row lock | The takeover `UPDATE` re-checks `lease_until <= now` against the row it finally locks, not the row it read, so a lease renewed during the wait is not stolen | The window lives between two statements inside one `reserve()`. Forcing it open needs `SELECT … FOR UPDATE` held from outside while the contender blocks — not expressible over HTTP, and faking it would discard the only mechanism that makes it a test | `tests/fencing_window_postgres.rs` |

Why it was admitted, with the evidence: `reservation.rs` stated in its own comment
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

Scenario 4 is the one issue #275 calls the highest-value invariant in the
backlog: today it is guarded by nothing.

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

Requires a reachable Docker daemon:

```bash
colima start                                             # or Docker Desktop
export DOCKER_HOST="unix://$HOME/.colima/default/docker.sock"
cargo test --manifest-path integration-tests/Cargo.toml
```
