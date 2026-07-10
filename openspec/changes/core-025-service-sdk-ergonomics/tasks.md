# Tasks: CORE-025 — Service SDK Developer Ergonomics

Reads `proposal.md`, `design.md` (v2 — AD-3 revised to a dedicated
`Injectable::validate()`, never trial-construct-and-discard `build()`), and
both spec deltas (`specs/service-sdk/spec.md`, `specs/testkit/spec.md`).
Every task below cites the design ADR and spec requirement/scenario it
satisfies. Line numbers were re-verified against the current source during
task breakdown (not copied blind from design.md); noted where they diverge.

Strict TDD Mode is active (`cargo test --workspace`). This is real new
behavior, not a deletion-heavy slice — every implementation task is preceded
by (or paired with) the failing test for its spec scenario. Do not write the
implementation before the test is red.

**Snapshot discipline (applies to every task touching `golden_codegen.rs`,
e.g. TASK-008, TASK-012):** do not run `cargo insta review`/`INSTA_UPDATE=1`
or otherwise accept a regenerated snapshot until `proxy_codegen.rs` (the
compile+run test of generated code) and the rest of the workspace's
compile-time tests are green first. A snapshot accepted too early can paper
over a real codegen regression — the failure looks like "snapshot mismatch,
approve the new one" instead of "the generated code is actually wrong."
Sequence: get the code compiling and the non-snapshot tests passing, only
then regenerate and manually review the snapshot diff line by line before
approving it.

**Ground-truth verification performed during breakdown** (grounding notes,
not tasks):
- `di/mod.rs`: `DepKey` at lines 76-85; `Injectable` trait at 87-99;
  `di_primitives_are_recognizable` test at 106-120 constructs all 4 variants.
- `runtime_builder.rs`: `DependencyTable::resolve_projection/adapter/config`
  at lines 69-93 (three `.ok_or(RuntimeError::DependencyNotFound)` sites);
  `RuntimeError` enum at 396-403; `RuntimeInner.resolved: DependencyTable` is
  a private field but `RuntimeInner` and `DependencyTable` live in the same
  module, so `RuntimeInner` methods can read `self.resolved.{adapters,
  configs,projections}` directly — no new accessor needed on `DependencyTable`
  itself for `check_dependency`.
- `builder.rs`: `RuntimeBuilder`/`Runtime` structs; `build()` at line 124;
  `matches!(result, Err(RuntimeError::DependencyNotFound))` at lines 422, 429.
- `resolvable.rs`: `Resolvable` trait at lines 44-57 (`type Proxy` only,
  no `type Service` yet).
- `registry/registry.rs`: `ServiceRegistry::register`/`resolve_raw` at
  78-121; `RegistryError::DuplicateService`/`ServiceNotFound` confirmed.
- `service-sdk-macros/lib.rs`: `classify_field_type` at line **596** (design.md
  cited 596-631 — confirmed exact); generated `Resolvable` impl at 471-484;
  `create_proxy`'s downcast-failure arm at line 481.
- `testkit/src/fixtures.rs`: hand-rolled `Injectable` at line 199-213,
  `DepKey::Config(TypeId::of::<u32>())` at line **205** (confirmed);
  `matches!(.., RuntimeError::DependencyNotFound)` at lines **255** and
  **331** (confirmed).
- `service-sdk/tests/proxy_codegen.rs`: `matches!(d, DepKey::Projection(_))`
  at line **254**, `matches!(d, DepKey::Adapter(_))` at line **255**
  (confirmed — both need `(_, _)`).
- `service-sdk/tests/golden_codegen.rs`: `golden_struct_dependencies_mixed`
  at line 109, `insta::with_settings!` filter at lines 116-121 (confirmed).
  **Correction to design.md's fallout list**: `golden_struct_dependencies_
  single_projection` (line 148-158) uses the **identical** filter block and
  snapshots a `DepKey::Projection(TypeId)` value — it will **also**
  regenerate once `DepKey` gains the type-name field, and needs the same
  filter extension. Design.md's AD-3 fallout only named the `_mixed` test;
  `_single_projection` is a second, real site. `golden_struct_dependencies_
  empty` (line 134-138) snapshots an empty `Vec<DepKey>` and is unaffected
  regardless of the variant shape change.
- `crates/service-sdk/examples/order_service.rs` is a **pre-existing**
  "manual equivalent" example (predates this change, does not use the real
  `#[service]` macro or any registration API) — task 19 below is a genuinely
  new file, not an edit to this one.

---

## Phase 1 — `RuntimeError::DependencyNotFound` becomes a struct variant (AD-4, F-03)

Atomic: changing the variant shape breaks every construction and `matches!`
site immediately. All four tasks in this phase must land together in one
compiling commit — there is no safe intermediate state.

### TASK-001: [x] `RuntimeError::DependencyNotFound { type_name, service_name }` + `Display` + `Error`
- File: `crates/service-sdk/src/runtime/runtime_builder.rs` (enum at lines 396-403).
- Change `DependencyNotFound` from a unit variant to
  `DependencyNotFound { type_name: &'static str, service_name: Option<&'static str> }`.
  Add `impl std::fmt::Display for RuntimeError` naming both fields (omit
  `service_name` gracefully when `None`) and `impl std::error::Error for RuntimeError {}`.
- Satisfies: service-sdk spec "Diagnosable Dependency Error" requirement, both scenarios.
- Test-first: add a test asserting `RuntimeError::DependencyNotFound { type_name: "X", service_name: Some("Y") }` formats naming both, and a test using it as `&dyn std::error::Error` (boxed via `?` into `Box<dyn Error>`) before finishing the impl.

### TASK-002: [x] Update the three `DependencyTable` construction sites
- File: `crates/service-sdk/src/runtime/runtime_builder.rs`, `DependencyTable::resolve_projection` (line 76), `resolve_adapter` (line 84), `resolve_config` (line 92) — **three sites, not two**; `resolve_projection` is easy to miss since projections are always empty from `with_registrations`, but the method still exists and must compile.
- Each `.ok_or(RuntimeError::DependencyNotFound)` becomes
  `.ok_or_else(|| RuntimeError::DependencyNotFound { type_name: std::any::type_name::<T>(), service_name: None })` (substitute `A`/`C` for the adapter/config arms).
- Depends on: TASK-001 (variant must exist first).

### TASK-003: [x] Fix every `matches!(.., DependencyNotFound)` call site
- Files/lines: `runtime/builder.rs:422,429` (test module); `runtime/runtime_builder.rs` tests at lines 430, 438, 445, 452, 510, 522, 534, 556-558 (7 assertions across `runtime_inner_default_creates_empty` and the `resolve_*_returns_not_found_for_*`/`concurrent_resolution_succeeds` tests); `testkit/src/fixtures.rs:255,331`.
- Each becomes `matches!(result, Err(RuntimeError::DependencyNotFound { .. }))`.
- Depends on: TASK-001.

### TASK-004: [x] Re-point `create_proxy`'s downcast-failure arm to `ServiceNotFound`
- File: `crates/service-sdk-macros/src/lib.rs:481` — `.map_err(|_| ego_service_sdk::runtime::RuntimeError::DependencyNotFound)?` becomes `.map_err(|_| ego_service_sdk::runtime::RuntimeError::ServiceNotFound)?`. A failed downcast is a resolution failure, not a missing dependency (AD-4 rationale).
- Must land together with TASK-001 — this call site would otherwise fail to compile the moment `DependencyNotFound` gains required fields.
- Acceptance: `cargo test -p ego-service-sdk --test proxy_codegen` and `--test golden_codegen` stay green; the descriptor snapshot is unaffected by this change (only the `Debug`-derived `DepKey` snapshots are touched, and only in Phase 2).
- Depends on: TASK-001.

---

## Phase 2 — `DepKey` gains a `&'static str` type-name field (AD-3, F-02 naming)

Also atomic for the same reason as Phase 1 — a public enum shape change.
Independent of Phase 1 (different enum), but touches two of the same files
(`di/mod.rs`, `testkit/src/fixtures.rs`) — sequence Phase 1 and Phase 2
back-to-back to avoid stacking merge risk, not because either strictly
requires the other.

### TASK-005: [x] Add the type-name field to all 4 `DepKey` variants
- File: `crates/service-sdk/src/di/mod.rs` (enum at lines 76-85).
- `Entity(TypeId)` → `Entity(TypeId, &'static str)`; same for `Projection`, `Adapter`, `Config`.
- Test-first: update `di_primitives_are_recognizable` (lines 106-120) to construct all 4 variants with a name argument (e.g. `DepKey::Entity(TypeId::of::<()>(), "()")`) BEFORE the enum shape change — confirm it fails to compile, then land the enum change.

### TASK-006: [x] Update macro `classify_field_type` (3 codegen arms)
- File: `crates/service-sdk-macros/src/lib.rs:596-631` (confirmed exact — function starts at line 596).
- At each of the 3 arms (`ProjectionRef`/`AdapterRef`/`ConfigValue`), add `std::any::type_name::<#inner_ty>()` as the second constructor argument alongside the existing `TypeId::of::<#inner_ty>()`. `Entity` is not macro-generated (no field type maps to it today) — no fourth arm needed here, only the enum variant itself (TASK-005) needs the field for API completeness.
- Depends on: TASK-005 (variant shape must exist first).

### TASK-007: [x] Fix remaining `DepKey` construction/match sites
- `testkit/src/fixtures.rs:205` — `DepKey::Config(std::any::TypeId::of::<u32>())` → `DepKey::Config(std::any::TypeId::of::<u32>(), "u32")`.
- `service-sdk/tests/proxy_codegen.rs:254-255` — **both** arms need updating: `matches!(d, DepKey::Projection(_))` → `matches!(d, DepKey::Projection(_, _))`, `matches!(d, DepKey::Adapter(_))` → `matches!(d, DepKey::Adapter(_, _))`.
- Depends on: TASK-005.

### TASK-008: [x] Regenerate and normalize the golden snapshots
- File: `crates/service-sdk/tests/golden_codegen.rs`.
- Extend the `insta::with_settings!` filter (currently at lines 116-121, and identically at 151-156) to also normalize `type_name`'s output to its trailing path segment (compiler-version-sensitive full paths would otherwise flake across toolchains) — e.g. add a filter pattern reducing `"some::module::path::TypeName"` to `"TypeName"` in the snapshotted string.
- Regenerate **both** affected snapshots: `golden_struct_dependencies_mixed` (line 109 — the one design.md's fallout named) AND `golden_struct_dependencies_single_projection` (line 148 — the one this task breakdown additionally found; same filter block, same `DepKey::Projection` shape change). `golden_struct_dependencies_empty` (line 134) is unaffected (empty vec).
- Run `cargo test -p ego-service-sdk --test golden_codegen`, review with `cargo insta review`, approve only the two dependency-shape snapshots — verify no other snapshot changed.
- Depends on: TASK-005, TASK-006.

---

## Phase 3 — `Injectable::validate()` + `RuntimeInner::check_dependency` (AD-3, F-02)

Depends on Phase 1 (needs the struct-variant `RuntimeError::DependencyNotFound`
to construct) AND Phase 2 (needs `DepKey`'s name field to read). This is the
sequencing constraint the proposal names explicitly.

### TASK-009: [x] Add the defaulted `Injectable::validate()` method
- File: `crates/service-sdk/src/di/mod.rs` (trait at lines 87-99).
- Add, per design.md's exact code:
  ```rust
  fn validate(rt: &crate::runtime::RuntimeInner) -> Result<(), crate::runtime::RuntimeError>
  where
      Self: Sized,
  {
      for dep in Self::dependencies() {
          rt.check_dependency(&dep)?;
      }
      Ok(())
  }
  ```
- A generic default — zero per-service codegen. `build()` (line 96-98) is untouched.
- Depends on: TASK-010 must exist for this to compile (or land together) — sequence them as one commit.

### TASK-010: [x] `RuntimeInner::check_dependency(&DepKey) -> Result<(), RuntimeError>`
- File: `crates/service-sdk/src/runtime/runtime_builder.rs` (new `impl RuntimeInner` method; `RuntimeInner` and `DependencyTable` share this module, so the method can read `self.resolved.{adapters,configs,projections}` directly).
- Per-kind `contains_key` against the resolved tables: `DepKey::Adapter(id, name)` → `self.resolved.adapters.contains_key(id)` (name only feeds the error); same for `Config`/`Projection`. **`DepKey::Entity(..)` arm MUST return `Err(RuntimeError::DependencyNotFound { type_name: name, service_name: None })` unconditionally** — no entity table exists, so this is a fail-safe default, not a bug.
- `service_name` stays `None` here; TASK-015 rewrites it to `Some(type_name::<S>())` on the way out of `try_build`.
- Visibility: at least `pub(crate)` — called cross-file from `builder.rs` (TASK-015).
- Test-first: unit tests for each of the 4 arms — adapter present/missing, config present/missing, projection present/missing (always missing per `with_registrations`), and `Entity` always `Err` regardless of table state — before wiring `validate()` to it.
- Depends on: TASK-001 (struct variant), TASK-005 (named `DepKey`).

---

## Phase 4 — `Resolvable::Service` associated type (AD-1/AD-2, F-01 prerequisite)

Independent of Phases 1-3; can run in parallel. Must land before Phase 5.

### TASK-011: [x] Add `type Service` to the `Resolvable` trait
- File: `crates/service-sdk/src/runtime/resolvable.rs` (trait at lines 44-57).
- Add `type Service: ?Sized + Send + Sync + 'static;` alongside the existing `type Proxy: Send + Sync;`.
- Satisfies: service-sdk spec's implicit contract for `with_service::<Tag>(Arc<Tag::Service>)` (next phase).

### TASK-012: [x] Emit `type Service = dyn #trait_name;` in the generated impl
- File: `crates/service-sdk-macros/src/lib.rs` (generated `impl ego_service_sdk::runtime::Resolvable for #tag_name` at lines 471-484).
- Add `type Service = dyn #trait_name;` beside `type Proxy = #ref_name;`.
- Acceptance: `cargo test -p ego-service-sdk --test golden_codegen` stays green (the snapshotted `descriptor()` output is unaffected — `type Service` is not part of `ServiceDescriptor`); `cargo test -p ego-service-sdk --test proxy_codegen` (a compile+run test) stays green.
- Depends on: TASK-011.

---

## Phase 5 — `RuntimeBuilder::with_service` / `Runtime::resolve` (AD-1/AD-2, F-01)

Depends on Phase 4 (`Tag::Service` must exist to compile the generic signature).

### TASK-013: `RuntimeBuilder::with_service::<Tag>(Arc<Tag::Service>) -> Result<Self, RegistryError>`
- File: `crates/service-sdk/src/runtime/builder.rs`.
- Per design.md's exact code: wrap the `Arc<Tag::Service>` in `ResolvableContainer`, coerce to `Arc<dyn Any + Send + Sync>`, call the existing `self.registry.register::<Tag>(<Tag as ServiceContract>::version(), raw)`.
- Satisfies: service-sdk spec "Canonical Service Registration" requirement, both scenarios (first registration succeeds; duplicate rejected via the registry's own `DuplicateService`, not silently overwritten).
- Test-first: write both scenarios' tests against a test-only service trait/tag (mirroring the `#[service]`-macro shape) before implementing.
- Depends on: TASK-012.

### TASK-014: `Runtime::resolve::<Tag>() -> Result<Tag::Proxy, RuntimeError>`
- File: `crates/service-sdk/src/runtime/builder.rs` (`impl Runtime` block).
- Calls `self.inner.registry.resolve_raw::<Tag>(&VersionConstraint::Exact(<Tag as ServiceContract>::version()))`, then **explicitly** maps `RegistryError::ServiceNotFound -> RuntimeError::ServiceNotFound` (any other `RegistryError` variant reached this way maps the same way — `resolve_raw` only returns `ServiceNotFound`) before calling `Tag::create_proxy(raw, self.inner.interceptor_chain.clone(), Arc::downgrade(&self.inner))`. This mapping step is explicit, not automatic from `resolve_raw`'s return type — implement it, do not skip it.
- Satisfies: service-sdk spec "Canonical Service Resolution Yields the Concrete Generated Proxy" requirement, all 3 scenarios (registered tag resolves and invokes identically to the hand-rolled path; unregistered tag → `ServiceNotFound`; tenant-scoped operation still fails closed through the same guard order).
- Test-first: write all 3 scenarios (including the tenant-scoped fail-closed one, reusing existing `#[tenant_scoped]` test fixtures) before implementing.
- Depends on: TASK-012, TASK-013 (registry must be populated by something for the positive-path test).

---

## Phase 6 — `RuntimeBuilder::with_injectable` / `try_build` (AD-3, F-02 terminal)

Depends on Phase 1 (struct-variant error) and Phase 3 (`validate()`/`check_dependency`).

### TASK-015: `with_injectable::<S: Injectable>(self) -> Self` + `try_build(self) -> Result<Runtime, RuntimeError>`
- File: `crates/service-sdk/src/runtime/builder.rs`.
- `with_injectable` records `S::validate` as a monomorphic `fn(&RuntimeInner) -> Result<(), RuntimeError>` (e.g. push into a `Vec<(&'static str, fn(&RuntimeInner) -> Result<(), RuntimeError>)>` pairing the fn pointer with `type_name::<S>()`).
- `try_build()` calls the existing infallible `build()` (line 124, **unchanged**), then runs every recorded validator against `rt.inner()`, returning the **first** failure with `service_name` rewritten to `Some(type_name::<S>())` (F-08's "report every missing dep" is explicitly deferred — first-failure semantics are correct for this slice). `Injectable::build` is never invoked during validation.
- Satisfies: service-sdk spec "Fail-Fast Dependency Validation at try_build()" requirement, all 3 scenarios.
- Test-first: write all 3 scenarios (missing adapter caught at `try_build()`; all deps present succeeds identically to `build()`; `build()` itself remains infallible and untouched by `with_injectable` bookkeeping) before implementing.
- Depends on: TASK-001, TASK-009, TASK-010.

### TASK-016: Regression check — CORE-018b's "`RuntimeBuilder::build()` Behavior Is Unchanged" tests
- Run the existing test suite covering that requirement (logger wiring, teardown ordering, security-provider installation scenarios in `runtime/builder.rs`'s test module) and confirm every one passes **unmodified** — zero edits to those tests' bodies or assertions.
- This is the explicit acceptance gate for TASK-015's claim that `with_injectable`/`try_build` do not alter `build()`'s existing contract (the proposal's Modified Capabilities section states this requirement stays as-is).
- Depends on: TASK-015.

---

## Phase 7 — TestKit pass-throughs (AD-5, F-06/F-07)

Depends on Phase 5 (`with_service`/`resolve` must exist to forward to).

### TASK-017: `FixtureBuilder::with_service::<Tag>(Arc<Tag::Service>) -> Result<Self, RegistryError>`
- File: `crates/testkit/src/fixtures.rs`.
- Thin pass-through: records the registration on the fixture's internal `RuntimeBuilder` (accumulated the same way `.config(..)`/`.authorization(..)` already are, before `build()` runs) by forwarding verbatim to `RuntimeBuilder::with_service`. No parallel `InterceptorChain`/`Weak` assembly.
- Satisfies: testkit spec "TestKit Trait-Proxy Registration and Resolution Use the Canonical Production Path" requirement, scenario 1.
- Test-first per that scenario before implementing.
- Depends on: TASK-013.

### TASK-018: `ServiceTestFixture::resolve::<Tag>(&self) -> Result<Tag::Proxy, RuntimeError>`
- File: `crates/testkit/src/fixtures.rs` (`impl ServiceTestFixture`, alongside the existing `service::<S: Injectable>()` at lines 78-80 and the `runtime()` accessor at 84-86 — note its doc comment "forward compatibility with a future public `Runtime::resolve`" is now stale and should be updated/removed since that future has arrived).
- Thin pass-through to `self.runtime.resolve::<Tag>()`. No bespoke proxy assembly.
- Satisfies: testkit spec requirement, scenarios 2 and 3 (same generated proxy + guard order as production `resolve()`; unregistered tag fails the same way with `ServiceNotFound`).
- Test-first per both scenarios before implementing.
- Depends on: TASK-014, TASK-017.

---

## Phase 8 — Minimal end-to-end example (F-09)

Depends on Phase 5 (registration/resolution) and Phase 6 (fail-fast DI), since
the Quick Path in design.md shows both flows.

### TASK-019: New example demonstrating the Quick Path
- File: `crates/service-sdk/examples/hello_service.rs` (new file — do not edit the pre-existing `order_service.rs`, which is a different, older "manual equivalent" example unrelated to this change).
- Uses the real `#[service]`/`#[operation]` macros (not a manual equivalent), demonstrates: `RuntimeBuilder::new().with_service::<HelloServiceTag>(Arc::new(HelloServiceImpl) as Arc<dyn HelloService>)?.build()`, then `rt.resolve::<HelloServiceTag>()?` and an invocation — matching design.md's Quick Path minimal-service snippet verbatim in spirit.
- Doubles as acceptance evidence per the proposal (F-09 "doubles as acceptance evidence that the happy path exists").
- Depends on: TASK-013, TASK-014.

---

## Phase 9 — End-to-end developer-journey acceptance pass (5 required scenarios)

Depends on every prior phase — this is a single realistic walkthrough, not a
restatement of the unit tests already written per-task above.

### TASK-020: One acceptance test file covering all 5 proposal scenarios in sequence
- File: `crates/service-sdk/tests/service_sdk_ergonomics_acceptance.rs` (new).
- One realistic developer-journey narrative, not isolated unit assertions:
  1. Minimal service (no deps): define, `with_service`, `build()`, `resolve()`, invoke.
  2. Service with dependencies (adapter + typed config via the existing `Injectable`/DI mechanism): `with_adapter`/`with_config`, `with_injectable`, `try_build()`, `Injectable::build(rt.inner())`, invoke.
  3. Missing dependency: the same DI service without a required adapter registered — `try_build()` fails with `DependencyNotFound { type_name, service_name }` naming both, caught at build time, not first invocation.
  4. TestKit: the same *kind* of service (a trait-proxy service, per F-06/F-07) constructed via `ServiceTestFixture::builder().with_service(..)` + `.resolve(..)`, proving no parallel wiring.
  5. Protected/tenant-scoped service: a `#[tenant_scoped]` service registered via `with_service` and resolved via `resolve`, invoked with a `ServiceContext` where tenant resolution fails — confirms the same `SecurityError`/guard order the hand-rolled path enforces.
- Depends on: TASK-014, TASK-015, TASK-018.

---

## Phase 10 — Documentation and scope-boundary checks

### TASK-021: `cargo doc --workspace --no-deps` — no new warnings
- Run `cargo doc --workspace --no-deps` and diff the warning count against the pre-existing baseline established since CORE-008B (that change's TASK-012 already ran this check and recorded zero errors — confirm no *new* warnings are introduced by this change's additive public surface: `with_service`, `resolve`, `with_injectable`, `try_build`, `Injectable::validate`, `RuntimeInner::check_dependency`, `Resolvable::Service`, the enriched `DepKey`/`RuntimeError::DependencyNotFound`).
- Depends on: all implementation tasks (TASK-001 through TASK-019).

### TASK-022: Explicit exclusion — do NOT touch `COOKBOOK.md`
- No code change. Recorded here so it is not done as an incidental drive-by: `COOKBOOK.md` (repo root) is explicitly **out of scope** for this change (proposal's F-05, deferred — "must sequence AFTER code lands, otherwise goes stale immediately"). Any cookbook rewrite documenting `with_service`/`resolve`/`try_build` is a separate, later change.
- No dependency; this is a standing note for the apply phase, not an executable task.

---

## Sequencing summary (compile-order dependency graph)

```
Phase 1 (RuntimeError struct variant, atomic) ──┐
Phase 2 (DepKey type-name, atomic)     ──┐      │
                                          ▼      ▼
                                   Phase 3 (validate/check_dependency)
                                          │
Phase 4 (Resolvable::Service, independent)     │
        │                                       │
        ▼                                       ▼
Phase 5 (with_service/resolve)          Phase 6 (with_injectable/try_build)
        │                                       │
        ▼                                       │
Phase 7 (TestKit pass-throughs)                 │
        │                                       │
        └──────────────┬────────────────────────┘
                        ▼
              Phase 8 (example) ──► Phase 9 (acceptance walkthrough) ──► Phase 10 (doc check / exclusion note)
```

Phase 1 and Phase 2 can be developed in parallel with each other (different
enums) but both must independently be atomic internally. Phase 4 can be
developed in parallel with Phases 1-3. Everything else is sequential per the
arrows above.

---

## Review Workload Forecast

Estimated changed lines by logical group (implementation + its accompanying
tests, since Strict TDD means tests land with their code):

| Group | Phases | Files touched | Est. lines changed | Nature |
|---|---|---|---|---|
| `RuntimeError` struct variant + call sites | 1 | `runtime_builder.rs`, `builder.rs` (tests), `fixtures.rs`, `service-sdk-macros/lib.rs` | ~70 | New `Display`/`Error` impl + ~13 mechanical call-site edits + new Display/Error tests |
| `DepKey` type-name + macro + snapshots | 2 | `di/mod.rs`, `service-sdk-macros/lib.rs`, `fixtures.rs`, `proxy_codegen.rs`, `golden_codegen.rs` (+ 2 `.snap` files) | ~65 | Enum field, 3 macro-arm edits, 4 construction/match-site edits, 2 snapshot regenerations + filter extension |
| `validate()`/`check_dependency` | 3 | `di/mod.rs`, `runtime_builder.rs` | ~90 | New generic default method + new `RuntimeInner` method + ~6 new unit tests (4 kinds × present/missing, plus Entity-always-Err) |
| `Resolvable::Service` codegen | 4 | `resolvable.rs`, `service-sdk-macros/lib.rs` | ~10 | Two one-line additive type declarations |
| `with_service`/`resolve` + tests | 5 | `builder.rs` | ~130 | Two new public methods (~30 lines) + ~5 new tests covering both spec requirements' scenarios (~100 lines, incl. a tenant-scoped fixture) |
| `with_injectable`/`try_build` + tests | 6 | `builder.rs` | ~110 | Two new public methods (~35 lines) + regression re-run (no new lines) + ~3 new scenario tests (~75 lines) |
| TestKit pass-throughs + tests | 7 | `fixtures.rs` | ~90 | Two thin forwarding methods (~15 lines) + ~4 new tests (~75 lines) |
| Example | 8 | `examples/hello_service.rs` (new) | ~90 | New illustrative file, real macros |
| Acceptance walkthrough | 9 | `tests/service_sdk_ergonomics_acceptance.rs` (new) | ~150 | One narrative integration test covering 5 scenarios |
| Doc check / exclusion note | 10 | none (verification only) | 0 | Read-only |
| **Total** | | **~12 files across 3 crates + 1 new test file + 1 new example** | **~805** | Real new behavior, not deletion — every line is new logic or a new test, unlike CORE-008B's deletion-heavy slice |

- **Chained PRs recommended: Yes.** ~805 estimated changed lines, driven by
  genuinely new logic and its accompanying tests (Strict TDD means tests are
  not optional overhead here — they're load-bearing evidence). No single
  logical group alone exceeds ~150 lines, but the total crosses the 400-line
  budget by a wide margin.
- **400-line budget risk: High.** Unlike CORE-008B, this is not a
  "confirm the grep sweep is clean" review — every group introduces new
  branches, new public API surface, and new error-handling paths that a
  reviewer must actually trace (particularly Phase 3's `check_dependency`
  presence semantics and Phase 6's validator-recording/first-failure logic).
- **Suggested split** (four PRs, each independently buildable and testable
  per the sequencing graph above):
  - **PR1** — Phase 1 + Phase 2 (mechanical enum-shape changes, ~135 lines): the two atomic, independent "must land together internally" groups.
  - **PR2** — Phase 3 + Phase 4 (~100 lines): fail-fast validation machinery + the `Resolvable::Service` codegen prerequisite, both gating later phases.
  - **PR3** — Phase 5 + Phase 6 + Phase 7 (~330 lines): the user-facing registration/resolution/fail-fast API surface plus its TestKit pass-throughs — the heart of the change, reviewed together since they share the same acceptance criteria (Scenario 5, guard-order preservation).
  - **PR4** — Phase 8 + Phase 9 + Phase 10 (~240 lines): example + acceptance walkthrough + final checks, reviewable as "does the whole journey actually work end to end."
- **Decision needed before apply: Yes** — per the Review Workload Guard,
  surface this to the user before `sdd-apply`: proceed with the 4-PR chain
  above (recommended, matches the natural phase boundaries), a different
  split, or a single PR under `size:exception` (harder to justify here than
  in a deletion-heavy change, since every line is new logic).
