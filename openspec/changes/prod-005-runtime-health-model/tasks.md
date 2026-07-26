# Tasks: PROD-005 — Runtime Health Model

## Review Workload Forecast

| Field | Value |
|-------|-------|
| Estimated changed lines | ~700-850 (2 new domain/service-sdk/runtime/testkit files, 1 new lifecycle default method, 1 builder wiring diff, 1 deletion in `access.rs`, incl. tests) |
| 400-line budget risk | High |
| Chained PRs recommended | Yes |
| Suggested split | PR1 domain contract+values+fold → PR2 service-sdk registry+aggregator+lifecycle seam+liveness → PR3 provider contributor+access.rs removal+testkit |
| Delivery strategy | auto-forecast (not a recognized ask-on-risk/auto-chain/single-pr/exception-ok label — treated conservatively) |
| Chain strategy | feature-branch-chain (PR1→PR2→PR3); only the tracker merged to develop |

Decision needed before apply: Yes (resolved)
Chained PRs recommended: Yes
Chain strategy: feature-branch-chain (PR1→PR2→PR3)
400-line budget risk: High

> Status: all tasks below are IMPLEMENTED and merged to develop via PR #243
> (PR1 #239 · PR2 #240 · PR3 #241). Checkboxes reflect the shipped state.

### Suggested Work Units

| Unit | Goal | Likely PR | Focused test command | Runtime harness | Rollback boundary |
|---|---|---|---|---|---|
| 1 | `ego-domain::health` — value types, `HealthContributor`, `fold` | PR1 | `cargo test -p ego-domain health::` | N/A — pure domain unit, no runtime | Delete `health/mod.rs`; revert `lib.rs` mod/re-export lines |
| 2 | `HealthRegistry`/`HealthAggregator`, `LifecycleManaged::health_contributors`, builder wiring, liveness path | PR2 | `cargo test -p ego-service-sdk health:: implementation:: runtime::builder::` | reference-app boot with zero contributors registered (default path unaffected) | Delete `service-sdk/src/health/mod.rs`; revert `implementation.rs` default method; revert `builder.rs` collection diff |
| 3 | `ProviderHealthContributor`, `access.rs` readiness removal, testkit `StaticHealthContributor` | PR3 | `cargo test -p ego-runtime providers::health:: && cargo test -p ego-testkit health::` | reference-app with a registered provider, aggregated via the single model | Delete `providers/health.rs`; restore `readiness()`/`ProviderSubsystemReadiness` in `access.rs`; delete `testkit/src/health.rs` |

## Phase 1: Domain — Health Contract, Values, Fold

- [x] TASK-001 RED: failing tests in `crates/domain/src/health/mod.rs` for the probe-independent per-contributor `fold(...)`: `Required`+`Unhealthy`⇒`Unhealthy`; `Optional`+`Unhealthy`⇒`Degraded` (never global `Unhealthy`); order-independence (same multiset, different order ⇒ identical result); empty set ⇒ `Healthy`. The fold takes NO `ProbeKind`; aggregation runs the SAME fold for every probe and only tags `HealthReport.probe` (TASK-026).
- [x] TASK-002 GREEN: implement `ProbeKind`, `HealthStatus`, `DependencyRequirement`, `HealthCode` (indicative closed set: `Timeout`, `Unavailable`, `InitializationPending`, `DependencyFailure`, `InternalFailure` — design proposal, refinable), `HealthCheck { status, code: Option<HealthCode> }` (no free-text field), `ContributorReport`, `HealthReport`, and `fn fold(...)` as the commutative/associative max-lattice (probe-independent; no `ProbeKind` parameter). AC: TASK-001 green.
- [x] TASK-003 RED: failing test proving (a) `HealthContributor` is object-safe (`Vec<Arc<dyn HealthContributor>>` built from a local trivial stub); and (b) a type/API-contract check that the public failure surface is ONLY `Option<HealthCode>` over a CLOSED `HealthCode` enum with NO string-carrying variant (explicitly no `HealthCode::Other(String)`/`Unknown(String)`) — the closed-set guarantee lives in the type, replacing the prior synthetic "struct never adds `message: String`" compile-test. Exhaustive failure→structured-code mapping is covered by the `ProviderHealthContributor` mapping tests (TASK-020).
- [x] TASK-004 GREEN: implement object-safe `#[async_trait] HealthContributor { fn name(&self) -> &str; fn requirement(&self) -> DependencyRequirement; async fn check(&self) -> HealthCheck; }` in `ego-domain` — **no liveness method**. AC: TASK-003 green.
- [x] TASK-005: wire `pub mod health;` + re-exports (`ProbeKind`, `HealthStatus`, `HealthCode`, `DependencyRequirement`, `HealthCheck`, `ContributorReport`, `HealthReport`, `HealthContributor`, `fold`) in `crates/domain/src/lib.rs`. AC: `ego_domain::{...}` importable; `cargo build -p ego-domain` succeeds.

## Phase 2: Service-SDK — Registry, Concurrent Aggregator, Liveness

- [x] TASK-006 RED: failing test in new `crates/service-sdk/src/health/mod.rs` — `HealthRegistry::register` + `HealthAggregator::readiness()` folds two registered stub contributors deterministically, matching the spec's "same inputs ⇒ same aggregate" scenario. AC also asserts the aggregator exposes ONLY `readiness()`/`startup()` entry points — there is no `aggregate(ProbeKind)` that could accept `ProbeKind::Liveness`.
- [x] TASK-007 GREEN: implement `HealthRegistry`, `HealthAggregator` (with distinct `readiness()` and `startup()` entry points as the ONLY aggregatable probes — no `aggregate(ProbeKind)` API), `HealthAggregationConfig`, fanning out over `FuturesUnordered`. AC: TASK-006 green.
- [x] TASK-008 RED: failing `#[tokio::test]` — a slow contributor's check does not delay the others; fast contributors' results are available without waiting on the slow one (assert via elapsed-time bound, not sequential ordering).
- [x] TASK-009 GREEN: implement concurrent fan-out via `FuturesUnordered` (replaces sequential-poll semantics). AC: TASK-008 green.
- [x] TASK-010 RED: failing `#[tokio::test]` — a contributor whose check never completes within its per-contributor timeout resolves to `HealthCheck { Unhealthy, Some(HealthCode::Timeout) }`; aggregation completes without hanging; other contributors unaffected.
- [x] TASK-011 GREEN: wrap each contributor's `check()` in `tokio::time::timeout(per_contributor)`; map `Err(Elapsed)` to `Timeout`. AC: TASK-010 green.
- [x] TASK-012 RED: failing `#[tokio::test]` — an optional global `HealthAggregationConfig::global_budget` bounds the whole join; contributors that COMPLETE before the deadline preserve their ACTUAL `ContributorReport`, while every contributor STILL PENDING at budget expiration receives a SYNTHETIC `ContributorReport { name, requirement, status: Unhealthy, code: Some(HealthCode::Timeout) }`. AC: the global timeout MUST NOT collapse aggregation into a single error — each unfinished contributor's `name`/`requirement` identity is preserved in the report.
- [x] TASK-013 GREEN: implement the optional global-budget wrap around the full `FuturesUnordered` join, retaining each in-flight future's contributor identity/metadata (`name`, `requirement`) so unfinished contributors are synthesized into `ContributorReport { name, requirement, status: Unhealthy, code: Some(HealthCode::Timeout) }` while completed contributors keep their actual reports. AC: TASK-012 green — contributor identity is never lost on global timeout.
- [x] TASK-014 RED: failing test — liveness computation invokes zero contributors: given a `Required` contributor stub programmed to resolve `Unhealthy`, liveness is unaffected; assert structurally via the liveness function's signature taking no registry argument.
- [x] TASK-015 GREEN: implement `Runtime::liveness()` (ADR-4 — the RuntimeInner internal check in `crates/service-sdk/src/runtime/runtime_builder.rs`) that takes **no registry parameter**, consults **no** contributor, and returns `HealthReport { probe: ProbeKind::Liveness, .. }`. Liveness MUST NOT be reachable via the aggregator — no `aggregate(ProbeKind)` entry point exists, only `readiness()`/`startup()`. AC: TASK-014 green.

## Phase 3: Lifecycle Registration Seam

- [x] TASK-016 RED: failing test in `crates/service-sdk/src/implementation.rs` — a `LifecycleManaged` impl overriding `health_contributors()` returns a non-empty `Vec<Arc<dyn HealthContributor>>`; a component that does not override it returns an empty `Vec` and leaves aggregation unaffected.
- [x] TASK-017 GREEN: add `fn health_contributors(&self) -> Vec<Arc<dyn HealthContributor>>` to `LifecycleManaged` with default `Vec::new()` (non-breaking). AC: TASK-016 green; all existing `LifecycleManaged` implementors compile unchanged.

## Phase 4: Runtime Wiring — Builder Collects Contributors

- [x] TASK-018 RED: failing test in `crates/service-sdk/src/runtime/builder.rs` — `RuntimeBuilder::build()` collects every registered lifecycle component's `health_contributors()` into the single runtime-owned `HealthAggregator`; a component registering none leaves readiness aggregation unaffected.
- [x] TASK-019 GREEN: implement collection in `build()`, registering each returned contributor into one `HealthRegistry`/`HealthAggregator` owned by the built `Runtime`/`RuntimeInner`. AC: TASK-018 green.

## Phase 5: Provider Health Contributor + Parallel Surface Removal

- [x] TASK-020 RED: failing test in new `crates/runtime/src/providers/health.rs` — `ProviderHealthContributor` maps `ProviderHealth::Healthy ⇒ (Healthy, None)`, `ProviderHealth::Unhealthy ⇒ (Unhealthy, Some(HealthCode::DependencyFailure))`; `requirement()` defaults `DependencyRequirement::Required`.
- [x] TASK-021 GREEN: implement `ProviderHealthContributor` wrapping `Arc<dyn ExternalDataProvider>`, implementing `HealthContributor`. AC: TASK-020 green.
- [x] TASK-022 RED: failing `#[tokio::test]` — two or more `ProviderHealthContributor`s registered into one `HealthAggregator` are checked concurrently (fan-out), not sequentially, and a slow one times out with a structured code without blocking the others (adapts existing #234 provider-health fixtures).
- [x] TASK-023 GREEN: during the runtime construction phase (the single registration authority — ADR-7), wire `RuntimeBuilder::register_data_provider`/`build()` (`crates/service-sdk/src/runtime/builder.rs`) so the builder adapts and registers one `ProviderHealthContributor` per registered data provider into the one runtime-owned aggregator. This is the SAME construction phase that collects `LifecycleManaged::health_contributors()` (TASK-019) — NOT a second registration channel, and no subsystem registers directly against a mutable global aggregator. AC: TASK-022 green; #234 "registered = required" preserved via the `Required` default.
- [x] TASK-024: delete `readiness()`, `ProviderSubsystemReadiness`, and their tests (`readiness_is_ready_when_every_registered_provider_is_healthy`, `readiness_is_not_ready_when_a_registered_provider_is_unhealthy`, `readiness_of_an_empty_subsystem_is_trivially_ready`) from `crates/runtime/src/providers/access.rs`. AC: `cargo build -p ego-runtime` succeeds; grep confirms zero remaining references to `readiness()`/`ProviderSubsystemReadiness` workspace-wide.

## Phase 6: Startup vs Steady-State Distinction

- [x] TASK-025 RED: failing test — a not-yet-initialized contributor returns the SAME probe-independent `HealthCheck { Unhealthy, Some(HealthCode::InitializationPending) }` (its `check()` does not receive or branch on the probe). In the aggregated report it appears as (Required ⇒ global `Unhealthy` / Optional ⇒ global `Degraded`) with `ContributorReport.code == InitializationPending`, and is distinguishable from a real `DependencyFailure` at the SAME global status (a `Required`+`DependencyFailure` contributor is also global `Unhealthy` but carries `code == DependencyFailure`).
- [x] TASK-026 GREEN: implement aggregation as `aggregate(probe, reports)` = the SAME per-contributor fold for EVERY probe, tagging `HealthReport.probe`; NO probe-specific status remap. `InitializationPending` MUST NOT alter the lattice. AC (TASK-025 green) — encode the frozen table (all 5 rows): Required+initializing ⇒ global `Unhealthy`, code `InitializationPending`; Optional+initializing ⇒ global `Degraded`, code `InitializationPending`; Required+real failure ⇒ global `Unhealthy`, code `DependencyFailure`; Optional+real failure ⇒ global `Degraded`, code `DependencyFailure`; Healthy ⇒ global `Healthy`, code `None`. The fold is identical for `readiness()` and `startup()`; the only difference is the `ProbeKind` tag; `check()` never branches on probe.

## Phase 7: TestKit — Same-Contract Test Support

- [x] TASK-027 RED: failing test in new `crates/testkit/src/health.rs` — a `StaticHealthContributor` fixed to `Optional`/`Unhealthy`, registered into a real `HealthAggregator`, deterministically drives the aggregate to `Degraded`, matching production semantics exactly.
- [x] TASK-028 GREEN: implement `StaticHealthContributor { status, requirement, delay }` implementing `HealthContributor` (optional `delay` via `tokio::time::sleep` before returning); wire `mod health;` + `pub use health::StaticHealthContributor;` in `crates/testkit/src/lib.rs`. AC: TASK-027 green.

## Phase 8: Cross-Cutting Guarantees & Verification

- [x] TASK-029: grep-verify transport neutrality — no HTTP/gRPC/GraphQL/Kubernetes symbol referenced anywhere in `crates/domain/src/health/mod.rs` or `crates/service-sdk/src/health/mod.rs`. AC: grep clean.
- [x] TASK-030: confirm acyclic layering — `ego-service-sdk` depends on `ego-runtime`; `ego-runtime` has no dependency back on `ego-service-sdk` (inspect `Cargo.toml` files). AC: dependency direction confirmed, no new cyclic edge introduced.
- [x] TASK-031: run `cargo test --workspace` and `cargo build --workspace`. AC: exit 0, no regressions.
- [x] TASK-032: confirm zero-contributor / zero-provider default runtime path is behaviorally unchanged (empty aggregation ⇒ `Healthy`, per fold's empty-set rule). AC: pre-existing test suite passes unmodified.
