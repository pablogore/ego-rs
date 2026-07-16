# Tasks: CORE-019A — External Data Providers SPI

Strict TDD Mode is enabled for this repo (`cargo test --workspace`). Every
phase below writes the RED test(s) first, then the minimal GREEN
implementation that satisfies them — never the reverse. Phases are ordered
bottom-up along the dependency edge `runtime → persistent-entity` (design.md
§4): the `persistent-entity` DTOs/port must exist before `runtime`'s SPI can
reference them, and `runtime`'s registry/access must exist before
`service-sdk`'s builder can wire them.

Each task cites the design.md AD(s) it implements and the spec.md
requirement it satisfies, so traceability runs task → AD → requirement
without re-deriving it later.

## Review Workload Forecast

| Field | Value |
|-------|-------|
| Estimated changed lines | ~595 (4 new runtime files, 1 new persistent-entity file, 1 modified service-sdk file, testkit + reference-app wiring, full test suite) |
| 400-line budget risk | Medium — total change exceeds 400 lines; no single suggested PR does |
| Chained PRs recommended | Yes |
| Suggested split | PR1 → PR2 → PR3 (see Work Units) |
| Delivery strategy | ask-on-risk → resolved: chained (3 PRs) |
| Chain strategy | stacked-to-main — PR2 targets PR1's branch, PR3 targets PR2's branch, PR3 merges to `develop` |
| Decision needed before apply | Resolved — see Suggested Work Units below |

### Estimated Lines By File/Task Group

| File | Action | Task group | Est. lines |
|------|--------|------------|-----------|
| `crates/persistent-entity/src/data_provider_access.rs` | Create | Phase 1 | ~90 |
| `crates/runtime/src/providers/provider.rs` | Create | Phase 2 | ~60 |
| `crates/runtime/src/providers/registry.rs` | Create | Phase 2 | ~90 |
| `crates/runtime/src/providers/access.rs` | Create | Phase 3 | ~140 |
| `crates/runtime/src/providers/mod.rs` | Create | Phase 3 | ~15 |
| `crates/service-sdk/src/runtime/builder.rs` | Modify | Phase 4 | ~70 |
| `crates/testkit/...` | Modify | Phase 5 | ~70 |
| `examples/reference-app/...` | Modify | Phase 6 | ~60 |
| **Total** | | | **~595** |

### Suggested Work Units

| Unit | Goal | Likely PR | Focused test command | Rollback boundary |
|---|---|---|---|---|
| 1 | `DataProviderAccess` port + DTOs (persistent-entity); `ExternalDataProvider` SPI + registry (runtime) — Phases 1-2 | PR1 | `cargo test -p ego-persistent-entity data_provider_access:: && cargo test -p ego-runtime providers::provider:: providers::registry::` | Delete `persistent-entity/src/data_provider_access.rs` and `runtime/src/providers/{provider,registry}.rs` — no other file touched |
| 2 | `RuntimeDataProviderAccess` chokepoint + module wiring (runtime); `register_data_provider` + teardown (service-sdk) — Phases 3-4 | PR2 | `cargo test -p ego-runtime providers:: && cargo test -p ego-service-sdk runtime::builder::` | Delete `runtime/src/providers/{mod,access}.rs`; revert `builder.rs` additions; PR1 untouched |
| 3 | Test doubles (testkit) + dogfood provider + E2E (reference-app) — Phases 5-6 | PR3 | `cargo test --workspace providers` | Revert testkit doubles + reference-app wiring; PR1/PR2 unaffected |

## Phase 1: `persistent-entity` — Handler-Facing Port + DTOs

Design refs: AD-001, AD-004, AD-009. Spec refs: `persistent-entity` delta —
"Handler-Reachable External Data Access", "Missing Registration Fails Closed
From the Handler's Perspective", "Fetch Attempts Are Observable" (signal
shape only — chokepoint itself is Phase 3), "Existing Handlers Unaffected".

- [x] 1.1 RED: unmodified `PersistentEntity` handler that never references
  `DataProviderAccess` compiles and its existing tests pass unchanged
  ("Existing Handlers Unaffected" scenario)
- [x] 1.2 RED: `DataProviderAccess` is object-safe (`Arc<dyn
  DataProviderAccess>` compiles) — mirrors `key_resolver.rs`'s
  object-safety test
- [x] 1.3 GREEN: `DataRequest { key: String, payload: Vec<u8> }`,
  `DataResponse { payload: Vec<u8>, cache_hit: bool }`,
  `DataProviderError { Transient(String), Fatal(String), NotFound { key },
  ProviderMissing { provider_id } }` (AD-007), `#[async_trait] trait
  DataProviderAccess: Send + Sync { async fn fetch(&self, provider_id: &str,
  request: DataRequest) -> Result<DataResponse, DataProviderError>; }`
  (`data_provider_access.rs`)
- [x] 1.4 Note (not a test): "SPI Isolated From Runtime Internals" is
  satisfied structurally by this phase's layering (the port + DTOs live in
  `persistent-entity`, which never depends on `runtime`) — proven end-to-end
  once Phase 6's dogfood provider compiles against only public SPI types, not
  by a standalone unit test here.

## Phase 2: `runtime` — Provider SPI + Registry

Design refs: AD-002, AD-004, AD-005, AD-006, AD-012. Spec refs:
`external-data-providers` — "Duplicate Registration Fails At Registration
Time", "Explicit, Non-Reflective Registration".

- [x] 2.1 RED: `ExternalDataProvider` is object-safe (`Arc<dyn
  ExternalDataProvider>` compiles); a `StaticDataProvider`-shaped test double
  calling `fetch` (via `#[tokio::test]`) returns the expected response —
  proves object-safety and result propagation only. No `block_on` sync
  bridge: `handle_command`/`apply_event`/`apply_events` are already async,
  so there is no synchronous call site to bridge (AD-006 correction, PR1
  review — cache-first was never a real constraint here)
- [x] 2.2 GREEN: `#[async_trait] trait ExternalDataProvider: Send + Sync {
  async fn fetch(&self, request: DataRequest) -> Result<DataResponse,
  DataProviderError>; async fn shutdown(&self) {} }` (`provider.rs`)
- [x] 2.3 RED: registering two providers under distinct `provider_id`s both
  succeed and each resolves independently; registering a second provider
  under an already-registered `provider_id` fails immediately and the first
  registration remains sole owner (both scenarios from "Duplicate
  Registration Fails At Registration Time")
- [x] 2.4 RED: a provider type that exists but was never registered fails to
  resolve exactly as an unregistered key would ("Explicit, Non-Reflective
  Registration" scenario) — proves no scanning/reflection path exists
- [x] 2.5 GREEN: `ExternalDataProviderRegistry` = `HashMap<String, Arc<dyn
  ExternalDataProvider>>`; `register(id, provider) -> Result<(),
  DuplicateProviderId>`, fail-closed at registration (`registry.rs`)

## Phase 3: `runtime` — Observability Chokepoint

Design refs: AD-003, AD-008, AD-009. Spec refs: `external-data-providers` —
"Fail-Closed Provider Resolution", "Fetch Observability Signals", the
cross-provider-isolation integration test (design.md §8); `persistent-entity`
delta — "Missing Registration Fails Closed From the Handler's Perspective",
"Fetch Attempts Are Observable".

- [x] 3.1 RED: resolving an unregistered `provider_id` through
  `RuntimeDataProviderAccess::fetch` returns
  `DataProviderError::ProviderMissing`, never a silent default or empty
  value ("Fail-Closed Provider Resolution" / "Missing Registration Fails
  Closed" scenarios)
- [x] 3.2 RED: a successful fetch emits one `tracing` event carrying
  `provider_id`, a hashed `key` (never the raw key or `payload`), latency,
  `cache_hit`, and an explicit `outcome: ProviderOutcome` field
  (`Success | NotFound | Transient | Fatal | ProviderMissing`) derived once
  at the chokepoint (AD-008) — captured via an in-file `tracing::Subscriber`
  test double, following the same pattern as CORE-019's
  `effects/observability.rs` test
- [x] 3.3 RED: **cross-provider isolation** — two `testkit` doubles
  registered under distinct `provider_id`s, given structurally identical
  `DataRequest`s, never cross-resolve; each fetch returns exactly its own
  provider's response (design.md §8 newly-added integration test)
- [x] 3.4 GREEN: `RuntimeDataProviderAccess` implementing
  `DataProviderAccess` — registry lookup, event emission, `ProviderOutcome`
  derivation (`access.rs`)
- [x] 3.5 GREEN: `crates/runtime/src/providers/mod.rs` — subsystem root,
  re-exports (`ExternalDataProvider`, `ExternalDataProviderRegistry`,
  `DuplicateProviderId`, `RuntimeDataProviderAccess`)

## Phase 4: `service-sdk` — Registration + Lifecycle Wiring

Design refs: AD-001, AD-006. Spec refs: "Zero Runtime Overhead When Unused",
"Explicit, Single-Owner Lifecycle".

- [x] 4.1 RED: a `RuntimeBuilder` with zero providers registered incurs no
  measurable startup cost attributable to this capability (no registry, no
  facade constructed) — "Zero Runtime Overhead When Unused" scenario
- [x] 4.2 RED: with ≥2 providers registered, runtime shutdown invokes each
  registered provider's `shutdown()` exactly once, through the one owning
  teardown path — "Explicit, Single-Owner Lifecycle" scenario
- [x] 4.3 GREEN: `RuntimeBuilder::register_data_provider(id, provider) ->
  Result<Self, DuplicateProviderId>`; conditional registry/facade
  construction (empty registry → no `RuntimeDataProviderAccess` built);
  `register_async_teardown` hook drives every registered provider's
  `shutdown()` (`builder.rs`)

## Phase 5: `testkit` — Deterministic Test Doubles

Design refs: AD-010. Spec refs: "Providers Replaceable By Deterministic Test
Doubles" (first scenario).

- [x] 5.1 RED: `StaticDataProvider` returns a canned `DataResponse` for every
  `fetch` call; `RecordingDataProvider` records each `DataRequest` it
  receives and is inspectable after the fact
- [x] 5.2 GREEN: `RecordingDataProvider` / `StaticDataProvider`
  implementations of `ExternalDataProvider` (`crates/testkit/...`)
- [x] 5.3 RED: a handler resolving through the facade behaves identically
  and deterministically when its registered provider is swapped for a
  `testkit` double, with zero handler code changes — "Test double swaps in
  without touching handler code" scenario

## Phase 6: `examples/reference-app` — Dogfood + E2E

Design refs: AD-010, AD-011 (KeyResolver relationship — reference only, no
retrofit), AD-012 (genericity guardrail — the dogfood is the forcing
function). Spec refs: "Reference-app handler never constructs a client
inline", `persistent-entity` delta — "Handler fetches external data during
command handling" (E2E proof).

- [x] 6.1 RED: E2E — a reference-app handler fetches external data through a
  registered provider (never an inline client) and receives the expected
  response, driven end-to-end through `RuntimeBuilder::register_data_provider`
  → the handler's `DataProviderAccess` facade
- [x] 6.2 GREEN: one trivial dogfood provider (`impl ExternalDataProvider`)
  registered and wired in `examples/reference-app`; handler code calls only
  the facade, never constructs a client inline
- [x] 6.3 Verify: grep-style regression check (mirrors CORE-019's
  `transport_agnostic_lint.rs` precedent) — no reference-app handler
  constructs an external client type directly; every external-data access
  path routes through the registered facade

## Deferred / Not Implemented This Slice

- **Tenant Isolation For Tenant-Scoped Fetches** (spec.md
  `external-data-providers` requirement) is **not implemented in this
  slice**. Design.md §11 Open Questions explicitly defers tenant-stamping
  until a first real tenant-scoped consumer exists (AD-012 guardrail); the
  current `DataRequest` shape (AD-004) carries no tenant field. Do not
  fabricate a no-op test to paper over this — the requirement is
  intentionally unimplemented pending a real consumer, and this is a gap to
  flag at verify/archive time, not to silently satisfy.
- **`openspec/specs/external-data-providers/` (the live canonical spec
  directory)** is populated by the archive step from this change's delta
  spec (`specs/external-data-providers/spec.md`, already written in the spec
  phase) — not a code task in `sdd-apply`'s scope. Noted here so it is not
  silently assumed done by this file.
- **Timeout/Retry Observability** (spec.md `external-data-providers`
  requirement) is **not implemented in this slice** — AD-007 adopts no
  retry/backoff policy this slice (a fetch is inline to command handling,
  so there is no delivery loop for a policy to drive), so there is no
  timeout or retry attempt for any signal to reflect. PR2 review (F-01)
  found this MUST unimplemented and unscoped in spec.md; spec.md now
  carries a dedicated "Timeout/Retry Observability (Deferred — Future
  Capability)" requirement recording the target contract for once a retry
  policy exists, and the Fetch Observability Signals requirement no longer
  claims timeout/retry as part of this slice's MUST-emit set.
