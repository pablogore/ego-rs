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

The suite as a whole has a wall-clock budget, from issue #275: **≤5 minutes
total, ≤1–2 minutes for any individual slice.** A run that exceeds it is not
finished, even if every invariant is covered. Compilation and execution are
reported separately — a suite that takes twenty minutes to compile and ninety
seconds to run has not broken the budget, but it has found the next thing worth
fixing.

## The four PROD-012 scenarios

Each one exercises the whole protocol; together they cover it. None of them is a
variant of another.

| # | Scenario | Guarantee it demonstrates | Why in-process cannot show it |
|---|---|---|---|
| 1 | Two identical `POST /register` | One execution, a durably completed response, and a replay served from PostgreSQL | The stored response has to survive a real commit and be read back through a real query — a scripted store returns whatever it was handed |
| 2 | Same key, different payload | Permanent conflict, with no second execution | The fingerprint comparison is a real uniqueness constraint under a real transaction, not an `if` in a test double |
| 3 | Recovery after an expired lease | Takeover under real fencing, without repeating steps already confirmed | Lease expiry is a clock-versus-row-state race resolved by the database; the receipt that stops the repeat was committed by a previous transaction |
| 4 | Two concurrent replicas | Exactly one obtains the permit; the other does not execute | Mutual exclusion between processes is what the `lease_until <= $N` guard exists for, and a single-process test cannot contend for it |

Scenario 4 is the one issue #275 calls the highest-value invariant in the
backlog: today it is guarded by nothing.

## Upstream blocker — what this suite cannot depend on yet

`ego-testkit`, `ego-service-sdk` and `reference-app` **cannot be dependencies of
this workspace today**, which means the four PROD-012 scenarios above are not yet
implementable here. The takeover/fencing invariant — issue #275's stated
priority, and the one thing in the backlog guarded by nothing — is, because it
needs only `ego-domain` and `ego-persistence`.

The cause is upstream. `kitlogger` declares:

```toml
kit-config = { path = "../../../kit-config/crates/kit-config", ... }
```

a relative path that escapes its own git repository. Cargo resolves a git
dependency's path dependencies *inside* that checkout, where `kit-config` does
not exist, so resolution fails with `no matching package named kit-config`.

**This is not specific to the nested workspace.** Measured: deleting the root
`Cargo.lock` and running `cargo generate-lockfile` at the root fails the same
way. The repository builds only because a committed lockfile already pins the
package — it cannot regenerate it. A workspace with no lockfile, like this one,
has nothing to fall back on.

Declaring `kit-config` from git in this manifest does **not** help: the failure
is kitlogger's path dependency, not a missing source.

Until the upstream manifest is fixed, adding any of the three crates turns
`cargo generate-lockfile` here into a hard error.

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
