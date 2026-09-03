# Design: STOOLAP-S1 — First-Class Stoolap `Repository` Adapter

> Canonical / source of truth. Spanish review companion: `design.es.md` (1:1 identifiers:
> EC-1..EC-7, AD-1..AD-11, S1..S3, OQ-1..OQ-3).
>
> **Inputs**: `proposal.md` (D-1..D-12, NG-1..NG-9, IS-1..IS-6, R1..R14, KD-1..KD-3, F-1..F-4,
> RK-1..RK-7) and the STOOLAP-S1 exploration (Engram `sdd/stoolap-s1/explore`, verdict **GREEN**).
> This document decides **how**: the schema and its tenant encoding, the durability DSN, the exact
> statement shapes and conflict-mapping logic, the crate's dependency closure and module tree, the
> shared conformance harness's signature and scenario set, and the slice boundaries. Observable
> requirements belong to `spec.md` and are not restated here.
>
> **Baseline read**: `develop` @ `e2bf2b4`. Every `file:line` below was read on this baseline, and
> every `stoolap` citation was read in the pinned crate source at
> `~/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/stoolap-0.4.0/` — not recalled from the
> inputs. Where the baseline contradicts an input, it is recorded as an **Evidence Correction**,
> not silently applied.

## Technical Approach

One new leaf-ward crate with four dependencies, one table, one unique index, one transaction shape,
and one shared conformance harness that three backends consume without any of them owning it.

The adapter is a **plain synchronous** implementation: `Repository` is a sync trait
(`crates/persistence-api/src/persistence/repository.rs:21-39`) and every `stoolap` call is sync, so
neither `PostgreSQLRepository::block_on` (`crates/persistence/src/postgres/repository.rs:51-53`) nor
`StoolapEffectStore::run_blocking` (`crates/effect-store/src/stoolap/mod.rs:227-236`) has a reason
to exist here. Both bridges solve a mismatch this crate does not have; D-4 already said so, and the
module tree below leaves no place to reintroduce either by reflex.

Two decisions carry the change, and both are settled here rather than at apply time. **D-5** replaces
PostgreSQL's two-partial-index tenant split with a NOT-NULL sentinel column plus one plain unique
index, because Stoolap skips uniqueness enforcement entirely on NULL. **D-6** hardcodes `sync=full`
into a single DSN constructor, because Stoolap's default does not fsync per commit and its DSN
parser fails *open* on an unrecognised sync value.

**No sequence diagram is included**, and that is an applicability call rather than an omission.
`openspec/config.yaml`'s design rule asks for one on complex async flows; this adapter has no async
flow at all and its longest call path is three statements inside one transaction, fully written out
under AD-5. The load-bearing structures here are the **schema** and the **conflict-mapping table**,
both given in full.

---

## Evidence Corrections

Seven. Each was found by reading the baseline or the pinned crate source rather than the inputs, and
each changes what the implementation must do.

### EC-1 — `InMemoryRepository` and `PostgreSQLRepository` **already disagree**, and the harness cannot cover the case they disagree about

This is the change's most consequential finding, and it lands exactly where the proposal predicted
it might (RK-5, NG-9, KD-3) — before any Stoolap code exists, which is what the Approach's ordering
was for.

For a **fresh** aggregate (no row / no entry) saved with a **non-zero** `expected_version`:

| Implementation | Behaviour | Evidence |
|---|---|---|
| `InMemoryRepository` | `current` is `0`, `current != expected_version`, so **`Conflict { expected, actual: 0 }`** | `crates/persistence-memory/src/persistence/repository.rs:40-48` |
| `PostgreSQLRepository` | `current_version` is `None`, so `new_version = 1` — `expected_version` is **never inspected** and the save **succeeds** | `crates/persistence/src/postgres/repository.rs:100-101` |

`save(id, agg, tenant, 42)` on an aggregate that does not exist therefore returns `Ok(1)` from one
shipped adapter and `Err(Conflict { expected: 42, actual: 0 })` from the other. Both satisfy the
trait signature. This is the same class of defect `crates/testkit/src/event_store.rs:1-20` was
written about, in the same port family, found the same way.

The trait's own documentation sides with the in-memory reading: *"`expected_version`: Optimistic
concurrency check. Use `0` for new aggregates."* (`persistence-api/src/persistence/repository.rs:18`).
An optimistic-concurrency check that is skipped precisely when the row is absent is not a check.

**Consequences, in order:**

1. **`StoolapRepository` implements the documented semantics** (AD-5): absent row + non-zero
   `expected_version` ⇒ `Conflict { expected, actual: 0 }`. Implementing the PostgreSQL behaviour
   deliberately, to make a harness pass, would be encoding a defect on purpose.
2. **The harness does not contain this scenario** (AD-8). Including it would fail against
   PostgreSQL, and NG-9/R11 forbid fixing a shipped adapter inside this diff. A conformance harness
   asserts what the implementations are required to agree on; a case where they demonstrably do not
   is a defect report, not a test to smuggle in.
3. **It is recorded as debt with a named follow-up** — KD-3 becomes concrete, and **F-5** is the
   change that reconciles them (see Named Follow-Ups). The harness gains the scenario there, not
   here.
4. **OQ-1** asks the user to confirm the direction (in-memory is canonical, PostgreSQL is the
   defect) before `sdd-tasks` writes the harness's scenario list. It is **non-blocking for the
   adapter** and **blocking only for whether F-5 is filed against PostgreSQL or against the trait
   documentation**.

### EC-2 — `ego-testkit` already depends on `ego-persistence-memory`, so the Memory harness run needs **zero** new dependency edges

The proposal's Affected Areas row gives `crates/persistence-memory/` a *"Dev-dependency + a test
target invoking the shared harness"*. That dependency already exists in the other direction:
`crates/testkit/Cargo.toml:20` — `ego-persistence-memory = { path = "../persistence-memory" }`, a
**normal** dependency.

So the Memory conformance run belongs in `crates/testkit/tests/`, where both the harness and
`InMemoryRepository` are already in scope. `crates/persistence-memory/` is then **not touched at
all** — no `Cargo.toml` edit, no test target, no source change — which is strictly better than the
proposal's plan and removes a `foundation → tooling` dev edge that would have been legal but
confusing (dev edges are excluded from the layer graph, so it would have passed while reading like
a violation). **AD-9.**

### EC-3 — Stoolap's DDL will accept a `PRIMARY KEY` and silently not enforce it

`crates/effect-store/src/stoolap/mod.rs:178-186` records this from direct experience against this
exact crate version: a `TEXT PRIMARY KEY` is **rejected at DDL time**, a table-level composite
`PRIMARY KEY (...)` is **parsed but silently not enforced** (no constraint, no index), and
`UNIQUE (...)` **is** fully enforced for both single and multiple columns.

The `aggregates`-equivalent table's identity is `(tenant_id, aggregate_id)` — a composite of two
`TEXT` columns, which is the intersection of both failure modes. Expressed as `PRIMARY KEY` it
would compile, run, and enforce nothing; every row would duplicate on every save. **The schema
therefore expresses identity exclusively through `UNIQUE`, and the word `PRIMARY KEY` does not
appear in this crate.** AD-2.

### EC-4 — `Database`'s registry keys on the **full DSN string**, so the DSN must have exactly one spelling

`DATABASE_REGISTRY` is a process-global `FxHashMap<String, Arc<DatabaseInner>>` keyed by the DSN
(`stoolap-0.4.0/src/api/database.rs:66-67`) and populated with `registry.insert(dsn.to_string(), …)`
(`:324`). Two `Database::open` calls naming the same directory with different query strings — say
`file:///data/db` and `file:///data/db?sync=full` — are two different keys, hence **two independent
engines over one directory**.

This is not hypothetical: `file://{path}` is exactly what the effect-store provider builds
(`crates/effect-store/src/stoolap/mod.rs:175`), so a second component opening the same directory the
"obvious" way would get its own engine. The mitigation is structural, not procedural: **one private
function builds the DSN, it takes only a path, and no call site anywhere in the crate assembles a
DSN by hand** (AD-4). Every handle for a given path is then byte-identical by construction.

### EC-5 — An unrecognised `sync` value fails **open**, silently

`parse_file_config`'s match arm is
`"none"|"off"|"0" => None, "normal"|"1" => Normal, "full"|"2" => Full, _ => SyncMode::Normal`
(`stoolap-0.4.0/src/api/database.rs:430-436`). There is no error branch. `sync=ful`, `sync=FULL `
with a stray space, or `sync=true` all resolve to `SyncMode::Normal` — the non-fsync default — with
no diagnostic anywhere.

This settles D-6's open sub-question decisively: **a config-driven or caller-supplied sync value is
worse than no knob at all**, because its failure mode is a silent durability downgrade that no
error, log line, or type can catch. `sync=full` is a hardcoded constant (AD-4).

### EC-6 — `Database::dsn()` is public, which gives the durability decision a real observable surface

`pub fn dsn(&self) -> &str` (`stoolap-0.4.0/src/api/database.rs:1131-1133`). R5's *"the adapter's
configured sync mode is asserted rather than assumed"* is therefore satisfiable by an assertion
against the handle the adapter actually opened, not merely against a string the test rebuilds
itself. AD-4's testing clause depends on this.

### EC-7 — The MVCC write-claim conflict has **no structured error variant**; it is `Internal` with a message

`VersionStore::try_claim_row` returns
`Err(Error::internal(format!("row {} has uncommitted changes from transaction {}", …)))`
(`stoolap-0.4.0/src/storage/mvcc/version_store.rs:4453-4473`). `Error::Internal { message: String }`
(`core/error.rs:286`) is a general-purpose variant; the dedicated `LockAcquisitionFailed(String)`
(`:199`) and `DatabaseLocked` (`:236`) variants that `backend_err` already classifies
(`crates/effect-store/src/stoolap/mod.rs:91-98`) are **not** used for this case.

So D-7's *"every write conflict maps to `Conflict`"* cannot be implemented by matching variants
alone: one arm has to match on message text. That is brittle, and the design does not pretend
otherwise — it is named, narrowed to a single arm, made **fail-loud** (unmatched errors stay
`Internal`, never become `Conflict`), and pinned by a test that races two real transactions so a
message change in a future Stoolap breaks the build rather than silently reclassifying every
concurrency conflict as an internal error. **AD-7.**

---

## Schema

One table, one composite unique index, four columns. Executed by the constructor on every open;
`IF NOT EXISTS` makes it idempotent, exactly as the effect-store provider's own DDL is
(`crates/effect-store/src/stoolap/mod.rs:187-219`).

```sql
CREATE TABLE IF NOT EXISTS aggregates (
    tenant_id    TEXT    NOT NULL,
    aggregate_id TEXT    NOT NULL,
    version      INTEGER NOT NULL,
    payload      TEXT    NOT NULL,
    UNIQUE (tenant_id, aggregate_id)
)
```

| Column | Type | Why |
|---|---|---|
| `tenant_id` | `TEXT NOT NULL` | The tenant scope in its sentinel encoding (AD-3). `NOT NULL` is the whole point: a nullable column would bypass uniqueness enforcement entirely (D-5). Listed first so column order matches index order |
| `aggregate_id` | `TEXT NOT NULL` | The caller's `&str` id, stored verbatim |
| `version` | `INTEGER NOT NULL` | Stoolap's `INTEGER` is `i64`-width, matching `Repository`'s `i64` version exactly — no narrowing anywhere |
| `payload` | `TEXT NOT NULL` | `serde_json::to_string(&aggregate)`. Stoolap has no JSON column type, and its `core::Value` has no binary variant either (the reason the effect-store provider base64-encodes bytes, `mod.rs:11-14`); a JSON **string** needs neither workaround |
| — | `UNIQUE (tenant_id, aggregate_id)` | The single index D-5 calls for. Inline rather than a separate `CREATE UNIQUE INDEX`: one statement, and this is the exact form the effect-store provider already proves works against stoolap 0.4.0 (`mod.rs:200,215`) |

**No `updated_at`.** PostgreSQL's `aggregates` carries one (`postgres/repository.rs:21`) because its
migration does; nothing in `Repository` reads it, this crate has no retention or audit port, and
carrying it would add `chrono` to the dependency set for a column no code path consumes. Omitted
deliberately, not overlooked — if an operator ever needs it, that is a schema change with a stated
reason. This is the same discipline NG-2 applies to abstractions, applied to a column.

**No `PRIMARY KEY`, anywhere.** EC-3.

**Rejected alternative — a separate index statement.** `CREATE UNIQUE INDEX IF NOT EXISTS
idx_aggregates_scope ON aggregates (tenant_id, aggregate_id)` is supported (the parser reads
`IF NOT EXISTS` for `CREATE INDEX` at `stoolap-0.4.0/src/parser/statements.rs:2084`) and would be
equivalent. Rejected only because it is a second statement with a second failure mode for no gain,
and the inline form is the one with in-tree evidence behind it.

---

## Architecture Decisions

### AD-1 — Crate, layer, and a four-entry dependency set with nothing speculative in it

**Decision** — `crates/persistence-stoolap/`, package `ego-persistence-stoolap`, one new line in
`layers.toml`'s `[layers]` table (currently `:15-36`) and one in the root workspace `members` list
(`Cargo.toml:2-24`):

```toml
"ego-persistence-stoolap" = "infrastructure"
```

`xtask/src/layers.rs` is **not opened**.

```toml
[package]
name = "ego-persistence-stoolap"
version = "0.1.0"
edition = "2021"

[dependencies]
# The only port this crate implements, plus `PersistenceError` and
# `resolve_tenant`. Not `ego-domain`: nothing here needs a domain value type
# (proposal D-3), unlike ego-persistence-memory which needed `Clock`.
ego-persistence-api = { path = "../persistence-api" }
# Already pinned at 0.4.0 in Cargo.lock via ego-effect-store's optional
# `stoolap` feature. No new external crate enters the workspace.
stoolap = "0.4"
# `Repository<A> for StoolapRepository<A, F>` bounds `A: Serialize` — the
# same bound PostgreSQLRepository carries (postgres/repository.rs:58).
serde = "1"
# `to_string` on the write path; `serde_json::Value` is the deserializer
# closure's input type, fixed by mirroring PostgreSQLRepository's `F`.
serde_json = "1"

[dev-dependencies]
# The shared conformance harness (AD-8). One direction only: ego-testkit has
# no dependency on this crate, so no cycle. Same shape as
# crates/effect-store/Cargo.toml:44.
ego-testkit = { path = "../testkit" }
# Every test needs a database directory of its own. Same reason and same
# version as crates/effect-store/Cargo.toml:39.
tempfile = "3"
```

**Criteria**:

1. **`infrastructure` is confirmed against the executable matrix, not assumed.** `layers.toml:10`
   permits `infrastructure → domain`, and `ego-persistence-api` is `domain` (`:17`), so the single
   outbound edge this crate creates is already legal and **no matrix arm changes**. The sibling
   precedents agree: `ego-persistence` (`:27`) and `ego-effect-store` (`:35`) are both
   `infrastructure`; `ego-persistence-memory` is `foundation` (`:36`) precisely because it drives no
   backend, and this crate does.
2. **The entry is mandatory, not optional.** `xtask`'s checks cover every workspace member whose
   manifest lives under `<root>/crates/`, so an unmapped `crates/persistence-stoolap/` fails
   FR-001's completeness check. Adding the entry *satisfies* `foundation-integrity`; it does not
   modify it — confirming the proposal's Capabilities note rather than assuming it.
3. **Four normal dependencies, each traceable to a line of code**, and nothing else:

   | Dependency | Required by | Evidence |
   |---|---|---|
   | `ego-persistence-api` | `Repository`, `PersistenceError`, `resolve_tenant` | `persistence-api/src/persistence/{repository.rs:12, error.rs:8, tenant.rs:29}` |
   | `stoolap` | `Database`, `Transaction`, `Error` | AD-4, AD-5, AD-7 |
   | `serde` | the `A: Serialize` bound on the `impl` | AD-5 |
   | `serde_json` | `to_string` (write path) and `Value` (the `F` closure's parameter) | AD-5, AD-6 |

4. **`async-trait` and `tokio` are absent on purpose** (D-4). No trait method here is `async`; the
   two in-tree async bridges (`postgres/repository.rs:51-53`,
   `effect-store/src/stoolap/mod.rs:227-236`) each solve a mismatch that does not exist in this
   crate. Their absence is a checkable diff property, not a promise.
5. **`chrono` is absent** because the schema has no timestamp column (see Schema).
6. **R7 is a grep, not a claim.** No `sqlx`, `PgPool`, `ego-persistence`, `postgres`, or migration
   token appears in the manifest or anywhere under `crates/persistence-stoolap/`.

### AD-2 — Module tree: one public type, one path to it

```
crates/persistence-stoolap/               (package: ego-persistence-stoolap, layer infrastructure)
├── Cargo.toml                            # AD-1
├── src/
│   ├── lib.rs                            # crate doc, `pub mod persistence;`, one root re-export
│   └── persistence/
│       ├── mod.rs                        # `pub mod repository;`
│       └── repository.rs                 # StoolapRepository + SYSTEMWIDE_SCOPE + encode_tenant
│                                         #   + dsn_for + is_write_conflict + colocated unit tests
└── tests/
    └── repository_conformance.rs         # the harness run (AD-9)
```

`lib.rs` carries `pub use persistence::repository::StoolapRepository;`. **Criteria**: the module path
mirrors `ego_persistence_api::persistence::repository` exactly, as `ego-persistence-memory`'s tree
does (its design AD-3), *and* the crate root re-exports its one public type, as `ego-persistence`
does (`crates/persistence/src/lib.rs:11`). The two precedents differ because they solved different
problems — persistence-memory had seven types across three port families and a compatibility matrix
to preserve; this crate has one type. With one type, one short path is the whole argument.

**No `#![deny(missing_docs)]`**, matching both sibling adapter crates
(`crates/persistence-memory/src/lib.rs:1`, `crates/persistence/src/lib.rs:1` — neither carries a
crate-level lint attribute). Doc comments are still written; this only declines to add a
workspace-inconsistent gate in a change whose subject is not lints.

**Everything except `StoolapRepository` is private.** `SYSTEMWIDE_SCOPE`, `encode_tenant`,
`dsn_for`, and `is_write_conflict` are crate-internal, which is what makes AD-3's non-leakage
argument structural rather than aspirational.

### AD-3 — D-5 resolved: the tenant sentinel, its one-way boundary, and why it cannot collide

**Decision** — three items in `repository.rs`, and one rule:

```rust
/// The systemwide (tenant-less) scope's on-disk spelling.
///
/// Stoolap skips unique-constraint enforcement outright when any indexed
/// column is NULL, and has no partial indexes — so PostgreSQL's two-partial-
/// index split (postgres/repository.rs:114-148, migration 015) has no
/// equivalent here, and a nullable `tenant_id` would let duplicate systemwide
/// rows accumulate for one aggregate with nothing raising an error.
///
/// `""` is safe as the sentinel because `resolve_tenant` rejects `Some("")`
/// as `MissingTenant` (persistence-api/src/persistence/tenant.rs:32) before
/// any adapter is reached, so the empty string can never arrive here as a
/// real tenant.
const SYSTEMWIDE_SCOPE: &str = "";

/// Encodes a resolved tenant scope into the value the `tenant_id` column
/// holds. The **only** place `Option<&str>` becomes a column value.
fn encode_tenant(resolved: Option<&str>) -> &str {
    resolved.unwrap_or(SYSTEMWIDE_SCOPE)
}
```

**The rule — no SQL statement in this crate ever selects `tenant_id`.** It appears only in `WHERE`
predicates and in `INSERT`'s column list. There is no decode direction because there is nothing to
decode into: no `Repository` method returns a tenant (`repository.rs:21-39` — `save` returns `i64`,
`load` returns `A`, `delete` returns `()`).

**Criteria**:

1. **The encode boundary is exactly one function, called in exactly three places** — the first two
   lines of `save`, `load`, and `delete`, each in the same shape:

   ```rust
   let resolved = resolve_tenant(tenant_id)?;          // MissingTenant escapes before any SQL
   let scope = encode_tenant(resolved.as_deref());     // the only Option -> column conversion
   ```

   No inline `unwrap_or("")`, no `.unwrap_or_default()` at a call site, no second spelling of the
   sentinel. This is the *"single narrow helper, not scattered inline logic"* requirement, and it is
   grep-checkable: `rg '""' crates/persistence-stoolap/src` must return exactly one non-test line,
   the `SYSTEMWIDE_SCOPE` declaration.

2. **`decode_tenant` is deliberately not written.** An unused inverse would be speculative code that
   exists only to make a symmetry argument, and it would *weaken* the non-leakage guarantee by
   creating the one code path that could leak. Non-leakage is instead **structural**: with
   `tenant_id` absent from every result set, there is no path from the stored column back to a
   caller, so R4 holds by construction rather than by review. If a future port ever needs to
   enumerate scopes, the decode direction is written then, with its consumer.

3. **The non-collision proof, in five steps** — each step is a fact about a line, not a judgement:

   1. Every value reaching `tenant_id` is `encode_tenant(resolve_tenant(caller_input)?.as_deref())`
      (criterion 1).
   2. `resolve_tenant` returns `Ok(None)` for `None`, `Err(MissingTenant)` for `Some("")`, and
      `Ok(Some(t))` for every other `Some(t)` (`tenant.rs:30-34`). Its `Some(t)` arm is therefore
      reachable only when `t != ""`.
   3. So `encode_tenant` yields `""` **iff** the resolved scope was `None`, and a non-empty string
      otherwise.
   4. The sentinel and the set of storable real tenants are therefore disjoint, and
      `Option<String> → String` is injective — two distinct scopes never share a `tenant_id` value,
      and one scope always produces the same one.
   5. Injectivity plus `UNIQUE (tenant_id, aggregate_id)` gives exactly one row per
      (scope, aggregate_id), **including the systemwide scope** — which is precisely the guarantee a
      nullable column would have silently dropped.

   Step 2 is the load-bearing one, and it is already tested upstream:
   `an_empty_tenant_is_rejected_rather_than_coerced_to_systemwide` (`tenant.rs:41-50`). This design
   **consumes** that test rather than duplicating it.

4. **Three test obligations, at three levels** (none of them a restatement of the others):

   | Level | Test | Proves |
   |---|---|---|
   | Crate unit | `encode_tenant_maps_only_the_absent_scope_to_the_sentinel` | Step 3 directly: `encode_tenant(None) == ""` and `encode_tenant(Some(t)) == t` for a non-empty `t` |
   | Crate unit (SQL) | `two_systemwide_saves_leave_exactly_one_row` | **R3**, the failure a nullable column would have permitted. Saves the same `aggregate_id` twice under `None`, then asserts `SELECT COUNT(*) FROM aggregates` is `1` and `version` is `2`. Row counting is adapter-internal, so this cannot live in the shared harness |
   | Shared harness | the three tenant scenarios (AD-8) | **R4**: behavioural indistinguishability across all three backends, including `MissingTenant` on `Some("")` |

5. **Rejected alternative — a random UUID sentinel.** It would remove the (already impossible)
   collision at the cost of an opaque magic string in every stored row and a sentinel with no
   provable relationship to the port's own rules. `""` is chosen *because* `resolve_tenant` already
   makes it unrepresentable, which is a proof rather than an improbability.

### AD-4 — D-6 resolved: `sync=full` is a hardcoded constant in one DSN constructor

**Decision** — one private function, one fallible constructor, no knob:

```rust
/// Builds the DSN this adapter opens for `path`.
///
/// `sync=full` is hardcoded, and this is the only DSN constructor in the
/// crate (see the criteria in design.md AD-4).
fn dsn_for(path: &Path) -> String {
    format!("file://{}?sync=full", path.display())
}

impl<A, F> StoolapRepository<A, F> {
    /// Opens (creating if absent) a durable Stoolap-backed repository at `path`.
    pub fn new(path: &Path, deserialize: F) -> Result<Self, PersistenceError> {
        let db = Database::open(&dsn_for(path)).map_err(internal_err)?;
        db.execute(CREATE_AGGREGATES_TABLE, ()).map_err(internal_err)?;   // Schema
        Ok(Self { db, deserialize, _marker: PhantomData })
    }
}
```

**Criteria**:

1. **Hardcoded, not a parameter and not config-driven** — the strongest of the three options, and
   the reason is EC-5, not taste. Stoolap's DSN parser has no error branch for an unrecognised
   `sync` value: `_ => SyncMode::Normal` (`database.rs:435`). A knob whose typo silently returns the
   adapter to the non-fsync default is worse than no knob, because the failure is invisible until a
   crash and no error, log, or type can surface it. A constant cannot be typo'd by an operator.
2. **A parameter was considered and rejected on customer grounds too.** The proposal's question
   round item 1 assumes durable single-node deployments, and D-6 states aggregate state is not a
   cache. No caller wanting the weaker mode has been identified, and adding the parameter *before*
   one exists is the same speculative move NG-2 rejects for abstractions. If one appears, it is a
   proposal with a stated customer — **F-6**.
3. **One DSN constructor closes EC-4.** Because `dsn_for` takes only a path and no call site
   assembles a DSN, every handle this crate opens for a given directory is byte-identical, so
   Stoolap's DSN-keyed registry (`database.rs:66-67,324`) hands back the *same* engine rather than a
   second one over the same files. A hardcoded query string and a single constructor are therefore
   the same decision seen from two angles.
4. **The constructor is fallible, and that is a justified divergence from IS-2's "mirroring
   `PostgreSQLRepository`'s public shape".** `PostgreSQLRepository::new` is infallible
   (`postgres/repository.rs:43`) because it receives an already-connected `PgPool` and the schema
   arrives via a migration run someone else owns. This adapter opens the database *and* owns its
   schema (IS-3), so both can fail. `-> Result<Self, PersistenceError>` reports that instead of
   panicking or deferring the failure to the first `save`. The generic shape — `<A, F>`, the
   `deserialize` closure, `Debug`, and the same trait bounds — is preserved exactly.
5. **The `Debug` impl prints the DSN, not the handle.** `PostgreSQLRepository`'s prints its pool
   (`:33-39`); ours prints `db.dsn()`, which is more useful and — per criterion 1 — contains no
   secret, only a path and a fixed sync mode.

**Testing — what is actually verifiable, stated honestly:**

| Ships | Test | What it proves |
|---|---|---|
| ✅ | `dsn_carries_full_sync` — pure function, `dsn_for(Path::new("/tmp/x"))` equals `"file:///tmp/x?sync=full"` | The exact string, including the `?` separator and the literal `full` that `database.rs:434` accepts |
| ✅ | `an_opened_repository_requested_full_sync` — asserts `repo.dsn()` (a thin accessor over `Database::dsn()`, EC-6) equals `dsn_for(path)` | The constructor actually used `dsn_for` — closing the gap between "the function is right" and "the function is called" |
| ✅ | `a_committed_save_survives_close_and_reopen` — save, drop the repository, reopen at the same path, load | Disk-backing and DDL idempotency. **Not fsync** |
| ❌ | a real crash-recovery / fsync test | See below |

**The fsync test is not written, and this is a deliberate refusal rather than a gap left open.**
The workspace does have real crash tests (`integration-tests/tests/infrastructure/single_aggregate_crash_recovery_postgres.rs`,
registered at `infrastructure.rs:117-122`), and a SIGKILL'd child is easy to arrange. But a killed
*process* loses nothing that reached the OS page cache — only a lost *machine* does. Such a test
would pass identically under `sync=none`, `sync=normal`, and `sync=full`: **a test that cannot fail
for the reason it claims to test is worse than no test**, because it converts an unverified property
into an apparently verified one. Genuinely proving fsync needs a fault-injecting filesystem or real
power loss, neither of which belongs in this change.

So R5's two halves are met differently and the design says which is which: *"a committed save
survives a close/reopen cycle"* is proven by the third test; *"the configured sync mode is asserted
rather than assumed"* is proven by the first two. That the fsync then genuinely happens is trusted
to Stoolap, and this line is the record of that trust being deliberate. **KD-4.**

### AD-5 — `save`: one real transaction, CAS on version, every lost race folded to `Conflict`

**Decision** — three statement constants and one algorithm. All statements are parameterized with
`$n` placeholders and tuple binding, the form the effect-store provider uses throughout
(`crates/effect-store/src/stoolap/mod.rs:416-417`); **no SQL string in this crate is ever built by
formatting a caller value into it**, so there is no injection surface (Threat Matrix).

```sql
-- SELECT_VERSION
SELECT version FROM aggregates WHERE tenant_id = $1 AND aggregate_id = $2

-- INSERT_AGGREGATE
INSERT INTO aggregates (tenant_id, aggregate_id, version, payload) VALUES ($1, $2, 1, $3)

-- UPDATE_AGGREGATE
UPDATE aggregates SET version = $1, payload = $2
 WHERE tenant_id = $3 AND aggregate_id = $4 AND version = $5
```

```
save(aggregate_id, aggregate, tenant_id, expected_version):

  1. resolved = resolve_tenant(tenant_id)?                     -> MissingTenant, before any SQL
     scope    = encode_tenant(resolved.as_deref())             -> AD-3
     payload  = serde_json::to_string(&aggregate)?             -> Internal on failure

  2. tx = db.begin()?                                          -> Internal on failure, never Conflict
                                                                  (ReadCommitted; see criterion 2)

  3. current: Option<i64> =
       tx.query(SELECT_VERSION, (scope, aggregate_id))?        -> first row, get::<i64>(0)
                                                                  Internal on failure

  4. new_version = match current:
       None                              -> 1                  if expected_version == 0
       None                              -> return Conflict { aggregate_id, expected: expected_version, actual: 0 }
       Some(c) if c == expected_version  -> expected_version + 1
       Some(c)                           -> return Conflict { aggregate_id, expected: expected_version, actual: c }
                                            (tx is dropped -> Stoolap auto-rollback)

  5. affected = match current:
       None    -> tx.execute(INSERT_AGGREGATE, (scope, aggregate_id, payload))
       Some(_) -> tx.execute(UPDATE_AGGREGATE, (new_version, payload, scope, aggregate_id, expected_version))
     on Err(e): is_write_conflict(&e) ? Conflict { expected: expected_version, actual: current.unwrap_or(0) }
                                      : Internal                                   -> AD-7

  6. if affected != 1:
       re-read via SELECT_VERSION inside the still-open tx     -> truthful `actual`
       return Conflict { aggregate_id, expected: expected_version, actual: re_read.unwrap_or(0) }

  7. tx.commit()?  -> same classification as step 5            -> AD-7
     Ok(new_version)
```

**Criteria**:

1. **Step 4's `None` arm implements the documented contract, and it is where this adapter
   deliberately does not copy PostgreSQL.** `postgres/repository.rs:100-101` returns `1` without
   inspecting `expected_version`; `InMemoryRepository` conflicts
   (`persistence-memory/src/persistence/repository.rs:40-48`); the trait documents *"use `0` for new
   aggregates"* (`persistence-api/src/persistence/repository.rs:18`). This adapter matches the
   documentation and the in-memory reading. **EC-1** carries the full finding and its consequences.
2. **Plain `begin()` (ReadCommitted), not `begin_with_isolation(SnapshotIsolation)`.**
   `Database::begin` defaults to `ReadCommitted` (`stoolap-0.4.0/src/api/database.rs:995-997`), and
   the CAS `WHERE version = $5` supplies the isolation this operation actually needs: any
   interleaving that changed the row invalidates the guard, so a stronger level would buy nothing
   and cost throughput. Snapshot isolation is available and rejected on those grounds, not
   overlooked.
3. **`FOR UPDATE` does not exist in Stoolap and is not emulated.** The dirty-write prevention
   `postgres/repository.rs:89-98` gets from a row lock comes here from MVCC write-claiming:
   `try_claim_row` refuses a second transaction's write to a row another transaction holds
   uncommitted (`version_store.rs:4453-4473`). Different mechanism, same property, and step 5's
   classification is what turns it into the same *caller-visible* result.
4. **Step 6 exists for a race step 4 cannot see.** Under `ReadCommitted`, a peer can commit between
   the SELECT and the UPDATE; the `WHERE version = $5` guard then matches nothing and `affected` is
   `0`. Re-reading inside the open transaction gives a **truthful** `actual` rather than a
   plausible-looking one, which matters because `Conflict`'s payload is what a retrying caller
   reloads against. If the row vanished entirely (a concurrent `delete`), `actual` is `0` — the same
   value a fresh aggregate reports, which is exactly what the store now holds.
5. **`Conflict.actual` in the write-claim case is the last committed version this transaction
   observed** (step 5's `current.unwrap_or(0)`), not a guess: the competing writer is uncommitted, so
   no later version exists to report yet. `Conflict { expected: 5, actual: 5 }` is a legitimate and
   honest outcome there, and the caller's response — reload and retry — is identical either way.
   Documented on the method so nobody reads it as a bug.
6. **`ON CONFLICT ... DO UPDATE` was considered and rejected**, even though the exploration
   confirmed Stoolap supports it with real conflict-target matching. With the sentinel column there
   is only one index, so it *would* work — but an upsert that overwrites cannot express *"only if
   the version is still `$expected`"*, which is the entire point of this method. Branching on the
   SELECT's result and guarding the UPDATE is both stronger and simpler, and it avoids depending on
   a dialect feature at all — a caution this workspace has already paid for once
   (`DELETE ... WHERE col IN (SELECT ... LIMIT n)` silently deletes zero rows in stoolap 0.4.0,
   `crates/effect-store/src/stoolap/mod.rs:242-251`).
7. **Rollback needs no explicit call on the error paths.** `Transaction` auto-rolls-back on `Drop`
   when neither `commit` nor `rollback` ran (`stoolap-0.4.0/src/api/transaction.rs:493-530` and its
   `Drop`), so every `return Err(...)` above is already transactional. Stated so nobody adds
   redundant `rollback()` calls that would then need their own error handling.

### AD-6 — `load` and `delete`: plain equality, because the column is `NOT NULL`

**Decision**:

```sql
-- LOAD_PAYLOAD
SELECT payload FROM aggregates WHERE tenant_id = $1 AND aggregate_id = $2

-- DELETE_AGGREGATE
DELETE FROM aggregates WHERE tenant_id = $1 AND aggregate_id = $2
```

`load`: `resolve_tenant` → `encode_tenant` → query → no row ⇒ `NotFound { aggregate_id }`; a row ⇒
`serde_json::from_str::<serde_json::Value>(&payload)` then `(self.deserialize)(value)`.
`delete`: same prologue → `execute` → `affected == 0` ⇒ `NotFound { aggregate_id }`, else `Ok(())` —
matching `postgres/repository.rs:205-209` exactly.

**Criteria**:

1. **`=` is correct here and `IS NOT DISTINCT FROM` would be noise**, which is the whole payoff of
   D-5. PostgreSQL needs the null-safe operator because its `tenant_id` is nullable and
   `NULL = NULL` is never `TRUE` (`postgres/repository.rs:82-88`; the incident
   `crates/testkit/src/event_store.rs:9-16` records). This column is `NOT NULL` and the systemwide
   scope is an ordinary value, so three-valued logic never enters — one operator, one predicate
   shape, both scopes.
2. **Neither statement selects `tenant_id`** — AD-3's rule, and the structural half of R4.
3. **`load` goes through `serde_json::Value` rather than straight to `A`.** The `F` closure's
   parameter type is fixed by mirroring `PostgreSQLRepository`'s (`postgres/repository.rs:59`), which
   receives a `Value` from `sqlx`'s JSON column. Deserializing `TEXT` → `Value` → `F` keeps a single
   deserializer usable against both backends unchanged, which is a precondition for the shared
   harness constructing all three the same way (AD-8).
4. **`delete` runs outside a transaction, deliberately.** It is one statement; wrapping a single
   statement in an explicit transaction adds two failure modes and changes no semantics.

### AD-7 — Error mapping: an explicit allowlist, a named brittle arm, and a fail-loud default

**Decision** — one private predicate, used only on the write path (`save`'s steps 5 and 7):

```rust
/// Whether a Stoolap error means "you lost a write race", as opposed to
/// "the backend failed".
///
/// The `Internal` arm matches on message text, and that is a known
/// brittleness, not an oversight: Stoolap's MVCC write-claim conflict has no
/// dedicated variant — `try_claim_row` returns
/// `Error::internal("row {} has uncommitted changes from transaction {}")`
/// (version_store.rs:4453-4473). `race_between_two_transactions_is_a_conflict`
/// pins it, so a future Stoolap changing this message breaks that test rather
/// than silently reclassifying every concurrency conflict as an internal error.
fn is_write_conflict(e: &stoolap::Error) -> bool {
    match e {
        stoolap::Error::UniqueConstraint { .. } => true,
        stoolap::Error::TransactionAborted => true,
        stoolap::Error::LockAcquisitionFailed(_) | stoolap::Error::DatabaseLocked => true,
        stoolap::Error::Internal { message } => {
            message.contains("uncommitted changes from transaction")
        }
        _ => false,
    }
}
```

| Stoolap error | → `PersistenceError` | Why |
|---|---|---|
| `UniqueConstraint { .. }` (`core/error.rs:104`) | `Conflict` | Two transactions both saw no row and both inserted; the D-5 index refused the second. A lost race, exactly |
| `Internal { message }` matching the write-claim text (`version_store.rs:4463`) | `Conflict` | The MVCC equivalent of losing a `FOR UPDATE` race — EC-7 |
| `TransactionAborted` (`:143`) | `Conflict` | The transaction lost; reload and retry is the correct response |
| `LockAcquisitionFailed(_)` (`:199`), `DatabaseLocked` (`:236`) | `Conflict` | See criterion 2 |
| **everything else** | `Internal(e.to_string())` | Corruption, schema mismatch, missing table, I/O failure — a caller must not retry these |
| any error outside `save`'s write path (open, DDL, SELECT, serialization) | `Internal` | `is_write_conflict` is never consulted there. A read that fails is not a race |

**Criteria**:

1. **The default direction is `Internal`, and that choice is load-bearing.** Callers retry on
   `Conflict`; mapping an unrecognised failure to `Conflict` would put a caller into a retry loop
   against a permanently broken backend, and the failure would never surface. Mapping a genuine race
   to `Internal` merely costs one avoidable error. Wrong in the safe direction, on purpose.
2. **`LockAcquisitionFailed`/`DatabaseLocked` map to `Conflict`, and the trade is named.**
   `backend_err` classifies both as `EffectStoreError::TemporarilyUnavailable`
   (`crates/effect-store/src/stoolap/mod.rs:91-98`) — retryable. `PersistenceError` has no retryable
   variant, and `Conflict` is its only variant callers already retry, so `Conflict` preserves the
   correct *behaviour* even though the word is imprecise. The alternative — `Internal` — turns a
   transient contention into a hard failure. The residual risk is honest: a permanently stuck lock
   presents as an endlessly retryable conflict, bounded only by the caller's own retry policy.
3. **No new `PersistenceError` variant is proposed, and this was checked rather than assumed.** Each
   Stoolap failure mode above lands in one of the four existing variants without distortion:
   `MissingTenant` is raised upstream by `resolve_tenant`, `NotFound` covers absent rows, `Conflict`
   covers every lost race, `Internal` covers backend failure. The one thing the four cannot express
   is *"transient, retry me"* as distinct from *"you lost a race"* — and criterion 2 shows the
   distinction has **no behavioural consequence for a `Repository` caller**, since both answers are
   reload-and-retry. Adding a fifth variant would reopen a shipped contract (NG-5, R6) to record a
   difference nobody acts on. **The four variants suffice; no API addition is requested.**
4. **The brittle arm is narrowed and pinned, not hidden.** It is one arm, on one variant, checked
   with `contains` on a substring stable across the format's parameters, and a colocated test races
   two real transactions on one row to assert `Conflict` — so the arm's correctness is verified
   against the pinned Stoolap rather than asserted about it.

### AD-8 — The shared harness: `assert_repository_conformance`, one concrete aggregate, eleven scenarios

**Decision** — `crates/testkit/src/repository_conformance.rs`, exported from
`crates/testkit/src/lib.rs` alongside its three siblings (`:38`, `:41`, `:45-48`, `:51-54`):

```rust
/// A minimal aggregate the harness owns, so all three backends are judged
/// against the same payload shape and construct the same deserializer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConformanceAggregate { pub value: String }

/// Builds a [`ConformanceAggregate`] carrying `value`.
pub fn conformance_aggregate(value: &str) -> ConformanceAggregate { … }

/// Asserts that a [`Repository`] implementation honours the parts of the
/// contract that are about *identity and versioning* — which rows belong to
/// which tenant scope, how a version advances, and what a lost race reports.
///
/// # Panics
///
/// Panics with a descriptive message on the first divergence from the contract.
pub fn assert_repository_conformance<R>(repository: &mut R)
where
    R: Repository<ConformanceAggregate> + ?Sized,
{ … }
```

**Criteria**:

1. **`crates/testkit/src/event_store.rs` is the closer template, not `ego-effect-store`'s
   `conformance.rs`** — D-9 already decided the *home*; this fixes the *shape*. Concretely inherited:
   the `assert_*_conformance` name, `&mut S` plus `?Sized`, one store instance with a distinct
   aggregate id per scenario (so the harness needs no way to build a fresh one — which matters
   because construction is fallible for Stoolap, infallible for Memory, and pool-dependent for
   PostgreSQL), panic-with-message on divergence, and a doc comment stating what is deliberately
   **not** checked.
2. **The aggregate type is concrete, where `event_store.rs`'s is generic.** That harness is generic
   over `E: DomainEvent` because `DomainEvent` is a trait with behaviour the contract depends on
   (`event_type()` is asserted). `Repository<A>`'s `A` has no trait and no behaviour — every
   property under test (tenant scoping, version arithmetic, error mapping) is entirely independent
   of it. A concrete `A` removes two bounds and a closure parameter, buys nothing away, and makes
   all three call sites near-identical. `integration-tests/.../repository_tenant_scoping_postgres.rs:37-53`
   already reached the same conclusion locally with its own `TestAggregate`; this hoists it.
3. **Owning `ConformanceAggregate` in `ego-testkit` is what makes one deserializer serve two
   backends.** `PostgreSQLRepository` and `StoolapRepository` both need
   `F: Fn(serde_json::Value) -> Result<A, PersistenceError>`; with `A` fixed by the harness, both
   call sites write the identical four-line closure (`:48-51` of the file above is already exactly
   that closure). `ego-testkit` already carries `serde` with `derive` (`Cargo.toml:16`), so no
   dependency is added.
4. **Eleven scenarios, each with its own aggregate id.** The first eight are the contract's core;
   the last three are `repository_tenant_scoping_postgres.rs`'s three tests generalized to any
   implementation, at that file's rigor bar (IS-5).

   | # | Scenario | Asserts |
   |---|---|---|
   | 1 | a fresh save starts at version 1 | `save(id, a, t, 0) == Ok(1)` |
   | 2 | sequential saves advance the version | `save(id, a, t, 1) == Ok(2)`, then `Ok(3)` |
   | 3 | a stale `expected_version` conflicts, truthfully | `Conflict { expected, actual }` with **both payload fields checked**, not just the variant |
   | 4 | load round-trips the aggregate | the loaded value equals the saved one, and the *second* save's value after an update |
   | 5 | loading an absent aggregate is `NotFound` | variant and `aggregate_id` |
   | 6 | deleting an absent aggregate is `NotFound` | same |
   | 7 | delete removes, and the load after it is `NotFound` | delete is real, not a tombstone |
   | 8 | `Some("")` is `MissingTenant` on **all three methods** | R4's caller-visible half; the sentinel is invisible |
   | 9 | the systemwide scope round-trips through save → load → save → delete | version `1` then `2` (a scope invisible to its own version check would return `1` twice), a stale conflict under `None`, `NotFound` after delete |
   | 10 | two tenants sharing an `aggregate_id` do not collide | independent rows, each its own value, and neither visible under `None` |
   | 11 | a tenant scope and the systemwide scope do not collide | independent rows, and deleting the systemwide one leaves the tenant-scoped one intact |

5. **Deliberately not covered, each for a stated reason** — a harness that asserts more than the
   contract turns every adapter into a copy of whichever one it was written against
   (`event_store.rs:47-51`):
   - **A fresh aggregate with a non-zero `expected_version`** — the two shipped implementations
     disagree (**EC-1**), and NG-9/R11 forbid fixing one here. Recorded as **KD-3/F-5**, and the
     scenario is added there. The harness's doc comment says this in full, so the omission is a
     documented finding rather than an invisible hole.
   - **Durability** — not in `Repository`'s contract; `is_durable()` does not exist on this trait.
     Pinned per-adapter instead (AD-4).
   - **Concurrency** — a shared harness cannot construct a second handle without knowing the
     backend. Pinned per-adapter (AD-7's race test).
   - **Payload shape** — the adapters are generic over `A`; asserting a serialization format would
     test `serde`, not the port.
6. **RK-4's ordering is a hard prerequisite, not advice.** The harness is written and green against
   the two existing implementations *before* any Stoolap code exists — which is exactly how EC-1 was
   found, and is the entire reason S1 is a separate slice (AD-11).

### AD-9 — Three call sites, and one of them costs no dependency at all

**Decision**:

| Backend | Run lives at | Dependency added | Why |
|---|---|---|---|
| `InMemoryRepository` | `crates/testkit/tests/repository_conformance_memory.rs` | **none** | `ego-testkit` already depends on `ego-persistence-memory` (`Cargo.toml:20`) — **EC-2**. `crates/persistence-memory/` is not touched at all |
| `StoolapRepository` | `crates/persistence-stoolap/tests/repository_conformance.rs` | `ego-testkit` + `tempfile`, both dev (AD-1) | Same shape as `crates/effect-store/Cargo.toml:44`. One direction; `ego-testkit` has no dependency on this crate, so no cycle |
| `PostgreSQLRepository` | `integration-tests/tests/infrastructure/repository_conformance_postgres.rs`, registered as one `mod` line in `integration-tests/tests/infrastructure.rs` | **none** | `integration-tests` already dev-depends on both `ego-testkit` (`Cargo.toml:59`) and `ego-persistence` (`:54`) |

**Criteria**:

1. **D-10 is upheld structurally.** PostgreSQL's run is in the separate workspace that owns the
   container (`integration-tests/Cargo.toml:1-15`); Stoolap's is embedded and file-backed, so it
   needs only a `tempfile` directory and stays in the root suite. **No Testcontainers or Docker
   dependency enters the root workspace** (NG-8, R9) — `cargo test --workspace` still passes with no
   container runtime available.
2. **Two of three call sites add nothing to any manifest**, which is what makes R13's *"declared
   once, consumed by every backend"* cheap rather than aspirational.
3. **Each call site is about five lines**: build the repository, call the harness. Identical modulo
   construction — which is itself evidence that the harness is judging the port rather than an
   implementation.
4. **`repository_tenant_scoping_postgres.rs` stays exactly as it is** (`infrastructure.rs:109-112`).
   Its three tests become a subset of what the harness covers, but it also documents *why* real
   PostgreSQL is required for them (`:12-22` — no in-memory double can misrepresent `NULL = NULL`),
   and deleting it would delete that reasoning. Overlap between a general harness and a
   backend-specific regression test is normal; NG-9 and R11 keep it untouched either way.

### AD-10 — The abstractions this design was tempted by, named and refused

The audit's rule is *no abstraction before two clear consumers*, and this change is where that rule
gets exercised rather than quoted. Four temptations arose while writing this document. Each is
recorded with what it would have abstracted and why it is refused **now** — not avoided silently.

| Temptation | What it would extract | Refused because |
|---|---|---|
| **A shared `TenantScope` encode/decode helper** in `ego-persistence-api`, since three adapters now each encode a tenant scope | `encode_tenant` + the sentinel constant | The three encodings are *genuinely different* — a `HashMap` key carrying an `Option`, a nullable SQL column read with `IS NOT DISTINCT FROM`, and a NOT-NULL sentinel column. A shared helper would have to serve all three and would collapse into `resolve_tenant`, which already exists and is already shared. The commonality is the *rule*, and the rule is already factored (`tenant.rs`) |
| **A `SqlRepository<D: Dialect>`**, since the PostgreSQL and Stoolap `save` bodies now visibly rhyme | statement text + the CAS algorithm behind a dialect trait | They rhyme and differ where it matters: `FOR UPDATE` vs. MVCC write-claim, partial indexes vs. sentinel, `IS NOT DISTINCT FROM` vs. `=`, async vs. sync, and — per EC-1 — *different version semantics*. A dialect trait would have to parameterize all five, at which point it is two implementations wearing a shared type. **NG-2, F-2** |
| **A `StoolapConnection` shared with `ego-effect-store`**, since both open a Stoolap database and both classify its errors | `Database::open` + DDL + error classification | The two want **opposite** things from both: this crate hardcodes `sync=full` while the effect store runs at the default (KD-2), and `backend_err` maps `LockAcquisitionFailed` to a *retryable* variant this port does not have (AD-7 criterion 2). Sharing would force one of the two to accept the other's durability and retry posture. Two consumers, two requirements, one wrong abstraction |
| **Generalizing the harness to `assert_port_conformance<P>`**, since `ego-testkit` now has four of them | the four `assert_*_conformance` functions behind one entry point | Four functions with four unrelated signatures and no shared call site. The generalization is a name, not a mechanism. **KD-1** already records that `Snapshot`, `OffsetStore`, and `DedupStore` still have none; writing those is what would produce evidence, and this is not that change |

**None of the four is forbidden forever.** F-2 states the condition: three concrete implementations
of a *second* port, with the duplication measured rather than predicted. One port is one data point,
and this workspace's own architecture-maturation rule needs two or three independent recurrences
before promotion.

### AD-11 — Three slices, in the order the dependencies and RK-4 both force

`sdd-tasks` owns task decomposition. This design owns only the boundaries, their order, and the
reason each boundary is where it is. The proposal's RK-7 forecast a 3-slice split; this confirms it
and supplies the seam argument, because the **harness genuinely can be written and proven green
before `StoolapRepository` exists** — that is not a convenience, it is what makes the harness a judge
rather than a mirror of the adapter (RK-4).

| Slice | Contents | New crate deps | RED |
|---|---|---|---|
| **S1 — the harness and its two existing subjects** | `crates/testkit/src/repository_conformance.rs` + one `mod`/`pub use` pair in `lib.rs`; `crates/testkit/tests/repository_conformance_memory.rs`; `integration-tests/tests/infrastructure/repository_conformance_postgres.rs` + its `mod` line | **none** (EC-2, AD-9) | The Memory test names `ego_testkit::assert_repository_conformance`, which does not exist yet |
| **S2 — the crate, its schema, and its statements** | `crates/persistence-stoolap/` (`Cargo.toml`, `lib.rs`, `persistence/{mod,repository}.rs`) with `new`/schema/`load`/`delete` and the full `save`; `layers.toml` entry; workspace member; the colocated unit tests (AD-3 criterion 4, AD-4, AD-7) | the AD-1 set | A crate-local test names `StoolapRepository`, which does not exist yet |
| **S3 — the third subject** | `crates/persistence-stoolap/tests/repository_conformance.rs`; the `ego-testkit` + `tempfile` dev-dependencies | dev only | The harness run fails to compile, then runs |

**Criteria**:

1. **S1 before S2 is forced by RK-4, not by size.** A harness written after the adapter is a harness
   written to whatever the adapter happens to do. S1's green run against Memory and PostgreSQL is
   what calibrates it — and it already paid for itself: **EC-1 is an S1-shaped finding**, surfaced at
   design time here precisely because the ordering was respected in analysis before it was respected
   in code.
2. **S2 is one reviewable unit even though it is the largest.** Splitting the schema from `save`
   would leave a crate whose only test is that a table can be created — a slice that proves nothing
   and still costs a review. The schema and the CAS algorithm are one decision (D-5 is *why* the CAS
   is expressible at all), and D-5 is the decision the reviewer most needs to see whole.
3. **An alternative seam was considered and rejected**: `new` + schema + `load`/`delete` as one
   slice, `save` as another. It splits at a real boundary — `save` is the only method with a
   transaction — but it lands the sentinel schema without the code that makes the unique index
   matter, so R3 (the systemwide-duplicate proof) could not be written until the following slice.
   **If S2 exceeds the review budget in practice, this is the seam to use**, with R3 moving to the
   second half. Recorded so the fallback is a decision rather than an improvisation.
4. **Every intermediate state compiles and every prior slice stays green.** After S1 the workspace
   has a harness and two passing subjects and no new crate. After S2 it has an unwired crate nothing
   depends on. After S3, three subjects. The proposal's mid-flight rollback property holds at each
   boundary, and `xtask/src/layers.rs` is never opened, so there is no gate state to unwind.
5. **Strict TDD is satisfiable at every slice.** Each RED is a compile failure naming a path that
   does not exist yet — the RED shape `ego-rs-testing-tdd` accepts.

---

## Integration Points

| Boundary | Direction | Mechanism | Verified at |
|---|---|---|---|
| `ego-persistence-stoolap` → `ego-persistence-api` | new, one-way | `path` dependency; `Repository`, `PersistenceError`, `resolve_tenant` | AD-1 |
| `ego-persistence-stoolap` → `stoolap` | new, one-way | crates.io `0.4`, already in `Cargo.lock` | AD-1 |
| `ego-persistence-stoolap` → any other workspace crate | **none** | no `path` dependency exists | AD-1 criterion 6; R7 |
| `ego-persistence-stoolap` → `ego-testkit` | new, one-way, **dev only** | harness consumption; excluded from the layer graph | AD-9 |
| `ego-testkit` → `ego-persistence-memory` | **unchanged** | already a normal dependency (`Cargo.toml:20`) | EC-2 |
| `integration-tests` → `ego-testkit`, `ego-persistence` | **unchanged** | already dev dependencies (`:59`, `:54`) | AD-9 |
| `crates/persistence-memory/**` | **untouched** | no manifest, source, or test change | EC-2, AD-9 |
| `crates/persistence-api/**` | **untouched** | no method, bound, supertrait, default body, or error variant changes | NG-5, R6; AD-7 criterion 3 |
| `crates/persistence/**` | **untouched** | no SQL, migration, index, or rename | NG-6, R7 |
| `crates/effect-store/**` | **untouched** | KD-2 is observed, not fixed | proposal KD-2 |
| `layers.toml` → `verify-layers` | in | one new entry, existing loader | AD-1 |
| `allowed_layers` → `check_direction` | **none** | no match arm changes; `infrastructure → domain` already permitted (`layers.toml:10`) | AD-1 criterion 1 |
| Production wiring | **none** | nothing constructs a `StoolapRepository`; a deployment opts in by adding the dependency | proposal Rollback Plan |

**No cycle is introduced, and this is a fact about files rather than a review promise.**
`ego-persistence-api` names no workspace `path` dependency, and `ego-persistence-stoolap` names only
it. The dev edge to `ego-testkit` runs into a `tooling` sink (`layers.toml:34`) that has no
dependency back on this crate. Cargo would refuse a cycle before `xtask verify-layers` ran, and
FR-003's cycle check would refuse it again — satisfying `openspec/config.yaml`'s *"No circular
dependencies between crates"* rule by construction.

## Testing Strategy

Strict TDD (`openspec/config.yaml` → `apply.tdd: true`). Every slice's RED is a compile failure
naming a path that does not exist yet (AD-11).

| Level | Location | What it proves |
|---|---|---|
| Shared harness (primary) | `crates/testkit/src/repository_conformance.rs` | The eleven scenarios of AD-8 — **R1, R2, R4, R13** |
| Harness run — Memory | `crates/testkit/tests/repository_conformance_memory.rs` | **R2** subject 1; calibrates the harness against a known-good implementation (RK-4) |
| Harness run — PostgreSQL | `integration-tests/tests/infrastructure/repository_conformance_postgres.rs` | **R2** subject 2, in the workspace that owns the container (**D-10, R9**) |
| Harness run — Stoolap | `crates/persistence-stoolap/tests/repository_conformance.rs` | **R1, R2** subject 3, in the root suite with no Docker |
| Crate unit | `encode_tenant_maps_only_the_absent_scope_to_the_sentinel` | AD-3 step 3 |
| Crate unit (SQL) | `two_systemwide_saves_leave_exactly_one_row` | **R3** — one row and version `2`; the failure a nullable column would have permitted |
| Crate unit | `dsn_carries_full_sync`, `an_opened_repository_requested_full_sync` | **R5** first half (**D-6**, EC-5, EC-6) |
| Crate unit | `a_committed_save_survives_close_and_reopen` | **R5** second half — disk-backing and DDL idempotency, explicitly **not** fsync (AD-4, KD-4) |
| Crate unit (concurrency) | `race_between_two_transactions_is_a_conflict` | **R12** and AD-7's brittle arm — two real transactions on one row, asserting `Conflict` not `Internal` |
| Crate unit | `a_stale_expected_version_is_a_conflict` | **R12**'s other half, at the adapter (the harness covers it too; this one is what fails first when `save` regresses) |
| Untouched | `crates/persistence-api/src/persistence/tenant.rs:41-50` | AD-3 step 2, consumed rather than duplicated |
| Untouched | `integration-tests/.../repository_tenant_scoping_postgres.rs` | **R11** — passes unmodified alongside the new harness run (AD-9 criterion 4) |
| Gate | `cargo run -p xtask -- verify-layers` | **R8** — mapped (FR-001), edge permitted (FR-002), no cycle (FR-003), isolated compile (FR-005), **no matrix edit** |
| Workspace | `cargo test --workspace` with no container runtime | **R9** |
| Suite | `cargo run --manifest-path integration-tests/Cargo.toml --bin run-suite` | the PostgreSQL run |

Five properties are **diff properties** — checked by reading the change, not by a test:

- **R6** — `crates/persistence-api/**` is absent from the file list.
- **R7** — `crates/persistence/**` is absent from the file list, and no `sqlx`, `PgPool`,
  `ego-persistence`, `postgres`, or migration token appears anywhere under
  `crates/persistence-stoolap/`.
- **R10** — the crate declares exactly one `impl … for StoolapRepository`, and no `trait` of its own.
- **AD-3 criterion 1** — `rg '""' crates/persistence-stoolap/src` returns exactly one non-test line;
  `rg 'tenant_id' crates/persistence-stoolap/src` shows it only in `WHERE` clauses, the `INSERT`
  column list, and the DDL — never in a `SELECT` list.
- **AD-1 criterion 4** — no `async`, `async_trait`, `block_in_place`, `spawn_blocking`, or `tokio`
  token appears in the crate.

## Threat Matrix

| Boundary | Exposure | Control |
|---|---|---|
| SQL construction | Five statement constants, all `$n`-parameterized with tuple binding | **No SQL string in this crate is built by formatting a caller value into it.** `aggregate_id`, the tenant scope, the payload, and the version are all bound parameters. Grep-checkable: no `format!` produces a statement anywhere in the crate |
| Tenant isolation | The systemwide scope shares one column with every real tenant | AD-3's injectivity proof plus `UNIQUE (tenant_id, aggregate_id)`. `resolve_tenant` fails closed on `Some("")` **before any SQL runs**, so a misconfigured empty tenant can never be filed into the shared systemwide partition (`tenant.rs:17-23`). Harness scenarios 8–11 assert this behaviourally across all three backends |
| Sentinel leakage | An adapter-internal encoding could reach a caller | Structural: no statement selects `tenant_id`, and no method returns a tenant (AD-3 criterion 2). R4 |
| Data at rest | Aggregate payloads land in a plaintext file at the operator's path | Unchanged posture — the same as the effect-store provider's Stoolap file and PostgreSQL's own data directory. This adapter adds no encryption and claims none; it is the deployment's disk to protect |
| Credentials | none | The DSN carries a filesystem path and a fixed `sync=full`. No password, token, or network endpoint exists to leak through `Debug` (AD-4 criterion 5) |
| Durability | A silent downgrade to non-fsync commits | AD-4: hardcoded constant (EC-5's fail-open parser makes a knob unsafe), one DSN constructor (EC-4), and two tests pinning it |

No routing, shell command, subprocess, VCS/PR automation, executable-file classification, or
process-integration boundary is involved. No auth path, JWT verification, or `CrossTenantPermit`
check appears in the diff.

## Migration / Rollout

**No migration.** The crate creates its own table in its own database file, via `CREATE TABLE IF NOT
EXISTS` on every open (AD-4). No existing store's data, schema, or migration is read, shared, or
modified in either direction. No PostgreSQL migration file is added, edited, or referenced.

**No feature flag and no phased rollout.** Unlike `ego-effect-store`, whose backends are optional
features (`crates/effect-store/Cargo.toml:46-50`) because that crate hosts *several* backends, this
crate **is** the backend — a deployment opts in by adding the dependency, and gating a single-backend
crate behind a feature of itself is ceremony with no consumer.

Rollback is the proposal's, unchanged, and available at each of AD-11's three boundaries: drop
`crates/persistence-stoolap/`, remove the workspace member and the `layers.toml` entry, drop the
harness from `ego-testkit` and its three call sites. Nothing outside the new crate depends on it;
no existing source file changes behaviour; `xtask/src/layers.rs` was never opened.

## Traceability

| Proposal / explore item | Resolved by | Note |
|---|---|---|
| D-1, IS-1 | AD-1, AD-2 | crate at `crates/persistence-stoolap/`, package `ego-persistence-stoolap` |
| D-2 | AD-1 criteria 1–2 | `infrastructure`, one `layers.toml` line, `xtask/src/layers.rs` untouched — confirmed against the matrix, not assumed |
| D-3 | AD-1 criterion 3 | exactly four normal dependencies, each traced to a line; `ego-domain` genuinely not needed |
| D-4 | AD-1 criterion 4, Technical Approach | no async bridge; absence is a checkable diff property |
| **D-5**, IS-3, R3, R4, RK-1 | **AD-3**, Schema, EC-3 | sentinel constant + one encode function + the no-`SELECT tenant_id` rule; five-step non-collision proof; exact DDL; `PRIMARY KEY` refused |
| **D-6**, IS-4, R5, RK-2 | **AD-4**, EC-4, EC-5, EC-6, KD-4 | hardcoded `sync=full` in one DSN constructor, with the fail-open parser as the reason; what is tested and what honestly is not |
| D-7, R12, RK-3 | **AD-7**, EC-7 | explicit allowlist, one named brittle arm pinned by a race test, fail-loud default; **no new error variant required** |
| D-8 | AD-5 | real transaction + version-guarded conditional write; `FOR UPDATE` absent and not emulated; `ON CONFLICT` considered and rejected |
| D-9, IS-5, R13 | **AD-8** | `assert_repository_conformance` in `ego-testkit`, `event_store.rs`'s shape, eleven scenarios, four stated exclusions |
| D-10, IS-6, R2, R9, NG-8 | **AD-9**, EC-2 | PostgreSQL in `integration-tests/` only; Memory in `ego-testkit`'s own tests at zero dependency cost; Stoolap in the root suite |
| D-11, NG-7, R7 | AD-1 criterion 6, Testing diff properties | grep-checkable, not asserted |
| D-12, NG-1, NG-2, NG-3, R10, RK-6 | **AD-10** | four abstractions named and refused with reasons, rather than silently avoided |
| NG-4, NG-6, F-3, F-4 | Integration Points | `crates/runtime/`, `crates/effect-store/`, `crates/persistence/` absent from the file list |
| NG-5, R6 | AD-7 criterion 3 | the four existing `PersistenceError` variants suffice; checked against every Stoolap failure mode, not assumed |
| **NG-9, R11, KD-3, RK-5** | **EC-1**, AD-5 criterion 1, AD-8 criterion 5, **OQ-1**, **F-5** | a real Memory/PostgreSQL divergence found at design time; Stoolap follows the documented contract; the scenario is excluded from the harness and filed as debt, not fixed here |
| R14, F-1..F-4 | Named Follow-Ups | carried forward, plus F-5 and F-6 opened by this design |
| **RK-7** | **AD-11** | three slices confirmed with the seam argument, plus a named fallback seam if S2 still exceeds budget |
| `config.yaml` "sequence diagrams" | Technical Approach | explicit N/A — no async flow; the schema and the mapping table are the load-bearing structures |
| `config.yaml` "no circular dependencies" | Integration Points | one outbound edge into a crate with no workspace dependency of its own |
| `config.yaml` "decisions with rationale" | AD-1..AD-11 | each carries criteria and, where one existed, the rejected alternative |

## Known Debt (added by this design)

- **KD-4** — `sync=full` is asserted at the DSN, and fsync itself is trusted to Stoolap rather than
  verified. AD-4 explains why the available test shapes cannot fail for the right reason. Recorded
  so the limit of R5's guarantee is on the record.

## Named Follow-Ups (added by this design)

- **F-5** — **Reconcile `save`'s fresh-aggregate semantics across the three implementations**
  (EC-1). `PostgreSQLRepository` ignores `expected_version` when no row exists; `InMemoryRepository`
  and (per AD-5) `StoolapRepository` conflict. Its own change, with its own tests and its own
  blast-radius review, per NG-9/R11. The twelfth harness scenario belongs to it.
- **F-6** — A selectable sync mode, **only** if a deployment appears that needs the throughput trade
  (AD-4 criterion 2). It must also solve EC-5's fail-open parsing, since a knob that silently
  degrades on a typo is not an acceptable shape.

## Open Questions

- [ ] **OQ-1 — EC-1: which fresh-aggregate semantics is canonical?** The trait documentation and
      `InMemoryRepository` say a non-zero `expected_version` on an absent aggregate is a conflict;
      `PostgreSQLRepository` silently accepts it. This design implements the documented reading and
      excludes the scenario from the harness (AD-8 criterion 5). **Non-blocking for the adapter and
      for all three slices.** Blocking only for how F-5 is filed: against `PostgreSQLRepository` as a
      defect, or against the trait documentation as the thing that is wrong.
- [ ] **OQ-2 — What concurrency does the spec promise?** Stoolap's process-global registry
      (`database.rs:66-67`) means two `StoolapRepository` handles on one path share one engine
      in-process, and its file lock governs cross-process access. The proposal's question round item
      2 assumed *"supported, with single-node concurrency characteristics stated honestly"*.
      `sdd-spec` needs the concrete claim: single-process-single-node, or multi-process-single-node.
      **Blocking for `spec.md`, not for this design** — AD-5's transaction shape is correct either
      way.
- [ ] **OQ-3 — AD-4's constructor is fallible**, diverging from IS-2's *"mirroring
      `PostgreSQLRepository`'s public shape"* (`-> Self` there, `-> Result<Self, PersistenceError>`
      here). The reason is that this adapter opens the database and owns its schema, so both can
      fail. Flagged so the divergence is a decision on the record rather than a discrepancy someone
      finds during review. **Non-blocking.**
