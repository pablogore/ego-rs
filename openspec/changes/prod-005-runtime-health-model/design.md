# Design: PROD-005 — Runtime Health Model

## Technical Approach

Domain owns the *contract and values*; service-sdk owns the *async execution*.
`ego-domain::health` defines `ProbeKind`, `HealthStatus`, `HealthCode`,
`DependencyRequirement`, the report/value types, and an object-safe
`#[async_trait] HealthContributor`. `ego-service-sdk::health` owns the single
`HealthRegistry` + `HealthAggregator` (concurrent fan-out, per-contributor
timeout, optional global budget, deterministic fold). The provider subsystem
(`ego-runtime`) stops exposing its own readiness surface and instead registers
one `ProviderHealthContributor` per provider into the single aggregator.
Layering stays acyclic: `domain <- runtime <- service-sdk` (verified:
service-sdk depends on ego-runtime, runtime never on service-sdk). Adapters
(HTTP/gRPC/…) are out of scope and consume the same `HealthReport`.

## Architecture Decisions

### ADR-1 (DECISION 1): `HealthContributor` async shape → **Option A**

**Choice**: object-safe `#[async_trait] HealthContributor` in `ego-domain`.
**Rejected**: (B) sync contract + external runner; (C) manual `Pin<Box<dyn Future>>`.
**Rationale (grounded in fact, not preference)**:

| Option | Tradeoff | Verdict |
|---|---|---|
| A `#[async_trait]` in domain | Matches the *existing* domain pattern verbatim — `DedupStore`, `ReadModelStore`, `OffsetStore`, `ProjectionProcessor` etc. are already object-safe `#[async_trait]` ports in `ego-domain`, and `async-trait` is already a domain dependency. `ExternalDataProvider::health()` is itself `async fn`. | **Chosen** |
| B sync contract | A health check consults live state (warm pool, cached JWKS, backing dep); forcing it sync makes the provider adapter unable to call the existing async `health()`, and pushes blocking/pre-caching onto implementors. Fights the async provider surface. | Rejected |
| C manual boxed future | Exactly what `#[async_trait]` desugars to, minus ergonomics, and diverges from the codebase's uniform macro convention. Pure churn. | Rejected |

The "don't contaminate `ego-domain`" concern is real but is about pulling a
**runtime (tokio)** into domain — `async-trait` pulls none. Domain holds only
the contract + values; the tokio-based executor lives in service-sdk. The
contributor trait exposes **only** `check()` (readiness/startup); it has **no
liveness method**, so liveness structurally cannot consult contributors.

### ADR-2 (DECISION 2): #234/#235 migration → **Remove the parallel surface; wrap the SPI**

**Choice**: keep the provider-facing SPI `ProviderHealth` unchanged (zero churn
for app implementors); **remove** the parallel subsystem model
(`ProviderSubsystemReadiness`, `RuntimeDataProviderAccess::readiness()`); add a
`ProviderHealthContributor` adapter (one per provider) that maps
`ProviderHealth → (HealthStatus, HealthCode)` and registers into the single
aggregator.
**Rejected**: *adapt* `ProviderHealth` into the rich model (needless SPI break —
app impls of `health()` would all change); *keep subsystem-internal* (leaves two
parallel readiness semantics, which the single-model mandate forbids).
**Rationale + concrete impact**: `readiness()`/`ProviderSubsystemReadiness` are
referenced **only** inside `crates/runtime/src/providers/*` (grep-verified — no
reference-app/testkit consumer exists yet; #234 explicitly deferred wiring), so
removal is near-zero blast radius and leaves no dual semantics. Signature
changes: `RuntimeDataProviderAccess::readiness()` deleted; `pub struct
ProviderSubsystemReadiness` deleted; `ExternalDataProvider::health` +
`ProviderHealth` retained. #234's "registered = required" rule is preserved by
defaulting each `ProviderHealthContributor` to `DependencyRequirement::Required`.
Migration path for any future/test consumer: query the aggregator's global
`HealthReport` (or a testkit helper) instead of the provider-only call.

### ADR-3: Deterministic aggregation is `aggregate(probe, reports)`

`aggregate(probe, reports)` performs the SAME per-contributor fold over
`(status, requirement)` for EVERY probe and only tags the resulting
`HealthReport.probe`. The fold is IDENTICAL for `readiness()` and `startup()`;
there is NO probe-specific interpretation or status-remap step. `check()` is
**probe-independent** — a contributor produces the same `HealthCheck` regardless
of which probe is aggregating, MUST NOT receive or branch on `ProbeKind`.

**Per-contributor fold (probe-independent, identical for both probes):**
severity lattice `Unhealthy > Degraded > Healthy`; `(Healthy,_)→Healthy`;
`(Degraded,_)→Degraded`; `(Unhealthy,Required)→Unhealthy`;
`(Unhealthy,Optional)→Degraded` (optional-unhealthy is clamped, never global
Unhealthy). Global = `max` over contributions (commutative, associative →
order-independent, so identical inputs always yield the identical aggregate;
empty set → Healthy).

**Frozen principle — `InitializationPending` never alters the lattice:**
`InitializationPending` does NOT alter the lattice rules. The contributor
continues to report `Unhealthy`; `DependencyRequirement` determines `Unhealthy`
vs `Degraded`, and `HealthCode::InitializationPending` preserves the "still
initializing" semantics. The Startup outcome is expressed via the contribution
to the global status plus the per-contributor `ContributorReport.code`, not via
a different global status. The frozen contract:

| Contributor during `startup()` | Contribution to global status | ContributorReport.code |
|---|---|---|
| Required + initializing | Unhealthy | InitializationPending |
| Optional + initializing | Degraded | InitializationPending |
| Required + real failure | Unhealthy | DependencyFailure |
| Optional + real failure | Degraded | DependencyFailure |
| Healthy | Healthy | None |

Satisfies constraint 4 and the determinism scenarios.

### ADR-4: Liveness cannot even be expressed as an aggregation call

The aggregator exposes only `HealthAggregator::readiness()` and
`HealthAggregator::startup()` — the ONLY aggregatable probes. There is NO
`aggregate(ProbeKind)` entry point, so `ProbeKind::Liveness` is structurally
unable to enter the aggregator. Liveness is computed by `Runtime::liveness()`
(the RuntimeInner internal check) which takes **no registry** and consults **no**
contributor. Combined with the contributor trait having no liveness method,
liveness can consult no contributor by three independent structural guarantees
(no liveness trait method; no registry parameter; no aggregation entry point that
names liveness) — not convention (constraint 1).

**Alternative considered**: an `AggregableProbe { Readiness, Startup }` type the
aggregator accepts, leaving `ProbeKind` only for report tagging. Rejected in
favor of the two explicit methods — equally prevents liveness from being
expressed as an aggregation call, and is simpler than a second enum.

### ADR-5: Concurrent execution (replaces `access.rs:341` sequential poll)

`FuturesUnordered` over contributors, each wrapped in
`tokio::time::timeout(per_contributor)`; `Err(Elapsed) → HealthCheck { Unhealthy,
Some(HealthCode::Timeout) }`. Each in-flight future retains its contributor
identity/metadata (`name`, `requirement`) so every result is attributable. An
optional `global_budget` wraps the whole join: contributors that COMPLETE before
the deadline preserve their ACTUAL `ContributorReport`; every contributor STILL
PENDING at global-budget expiration receives a SYNTHETIC
`ContributorReport { name, requirement, status: Unhealthy, code: Some(Timeout) }`.
A global timeout MUST NOT collapse aggregation into a single error that loses
contributor identity. No single contributor blocks the rest; the aggregator never
hangs (constraint 5).

### ADR-6: Startup vs steady-state readiness

`ProbeKind { Liveness, Readiness, Startup }` tags each report. Startup and
readiness produce the SAME `HealthStatus` and the SAME `ContributorReport`; the
only difference is the `ProbeKind` tag and the semantic moment of consumption
(startup gate vs steady-state readiness gate). A registered but
not-yet-initialized contributor returns the SAME probe-independent
`HealthCheck { status: Unhealthy, code: Some(HealthCode::InitializationPending) }`
regardless of which probe is aggregating — `check()` never branches on the probe.

`HealthCode::InitializationPending` is what distinguishes "not ready because
still starting" from "not ready because a dependency failed"
(`DependencyFailure`) — WITHOUT inventing a `Starting` fourth state. Both codes
sit at the SAME status (Required → global `Unhealthy`; Optional → `Degraded`);
the code on the `ContributorReport`, not a different global status, carries the
distinction (constraint 6).

### ADR-7: Single registration authority, multiple sources

The runtime is the single registration authority. Lifecycle components MAY
contribute contributors via `LifecycleManaged::health_contributors()`.
Runtime-owned facilities such as registered data providers are adapted and
registered by the builder during the same construction phase. No subsystem
registers directly against a mutable global aggregator.

**Rationale**: one authority prevents two competing registration paths (a
component-driven seam and a provider-driven seam racing the same mutable
aggregator). The builder's provider registration is the runtime construction
phase acting as the authority — not a second registration channel — and the
aggregator is not mutated by subsystems after construction.

## Data Flow

    Runtime::liveness():  RuntimeInner internal check ──▶ HealthReport(Liveness)   [no registry, no contributor]

    HealthAggregator::readiness() / ::startup():   [NO aggregate(ProbeKind) — liveness cannot enter here]
      HealthAggregator ──fan-out(FuturesUnordered)──▶ [C1.check() … Cn.check()]  (probe-independent; each timeout-bounded)
             │  provider Ci = ProviderHealthContributor(Arc<dyn ExternalDataProvider>)
             ├── per-contributor fold(status,requirement)   (identical for readiness/startup)
             └── tag ProbeKind (no status remap)  ─▶ HealthReport { ProbeKind, HealthStatus, Vec<ContributorReport> }

### Sequence: readiness aggregation

    Caller        HealthAggregator      C1(fast)     C2(slow/provider)     C3(pending)
      │─readiness()─▶│                                                       
      │              ├─spawn timeout(check)─▶│                              
      │              ├─spawn timeout(check)──────────▶│  (exceeds budget)    
      │              ├─spawn timeout(check)───────────────────────▶│         
      │              │◀── Healthy ───────────│                              
      │              │◀── Elapsed ⇒ (Unhealthy,Timeout) ──│  (does not block C1/C3)
      │              │◀── (Unhealthy,InitializationPending) ─────────│  (same check() for any probe)
      │              ├ per-contributor fold (identical for readiness/startup): max over contributions
      │              ├ tag ProbeKind (no status remap)
      │◀─ HealthReport(Readiness, status, [reports]) ─┤                     

## File Changes

| File | Action | Description |
|------|--------|-------------|
| `crates/domain/src/health/mod.rs` | Create | `ProbeKind`, `HealthStatus`, `HealthCode`, `DependencyRequirement`, `HealthCheck`, `ContributorReport`, `HealthReport`, `#[async_trait] HealthContributor`, `fn fold(...)` |
| `crates/domain/src/lib.rs` | Modify | `pub mod health;` + re-exports |
| `crates/service-sdk/src/health/mod.rs` | Create | `HealthRegistry`, `HealthAggregator` (fan-out + timeout + global budget), `HealthAggregationConfig`; runtime owns one instance |
| `crates/service-sdk/src/implementation.rs` | Modify | `LifecycleManaged::health_contributors(&self) -> Vec<Arc<dyn HealthContributor>>` default `Vec::new()` (non-breaking seam) |
| `crates/service-sdk/src/runtime/builder.rs` | Modify | Collect contributors on init; register each provider as a `ProviderHealthContributor` |
| `crates/runtime/src/providers/health.rs` | Create | `ProviderHealthContributor` impl `HealthContributor`; maps `ProviderHealth → (HealthStatus, HealthCode)` |
| `crates/runtime/src/providers/access.rs` | Modify | Delete `readiness()` + `ProviderSubsystemReadiness` (parallel surface removed) |
| `crates/testkit/src/health.rs` | Create | Same-contract `StaticHealthContributor { status, requirement, delay }` |

## Interfaces / Contracts

```rust
// ego-domain::health — zero infra/transport deps
pub enum ProbeKind { Liveness, Readiness, Startup }
pub enum HealthStatus { Healthy, Degraded, Unhealthy }
pub enum DependencyRequirement { Required, Optional }
// INDICATIVE proposal (spec left the variant set open, not frozen):
pub enum HealthCode { Timeout, Unavailable, InitializationPending, DependencyFailure, InternalFailure }
pub struct HealthCheck { pub status: HealthStatus, pub code: Option<HealthCode> } // no free-text
pub struct ContributorReport { pub name: String, pub status: HealthStatus,
    pub requirement: DependencyRequirement, pub code: Option<HealthCode> }
pub struct HealthReport { pub probe: ProbeKind, pub status: HealthStatus,
    pub contributors: Vec<ContributorReport> }

#[async_trait]
pub trait HealthContributor: Send + Sync {   // object-safe; NO liveness method
    fn name(&self) -> &str;
    fn requirement(&self) -> DependencyRequirement;
    /// Probe-independent: returns the SAME HealthCheck regardless of probe.
    /// Contributors MUST NOT receive or branch on ProbeKind. Aggregation runs
    /// the SAME fold for every probe and only tags HealthReport.probe;
    /// InitializationPending never alters the lattice.
    async fn check(&self) -> HealthCheck;
}

// ego-service-sdk::health — runtime-owned aggregation
impl HealthAggregator {
    // The ONLY aggregatable probes; there is NO aggregate(ProbeKind), so
    // ProbeKind::Liveness cannot be expressed as an aggregation call.
    // Both run the SAME fold and only tag HealthReport.probe; identical status,
    // identical ContributorReports. InitializationPending never alters the lattice.
    pub async fn readiness(&self) -> HealthReport; // ProbeKind::Readiness
    pub async fn startup(&self) -> HealthReport;   // ProbeKind::Startup
}
// Liveness lives on the runtime — no registry, consults no contributor (ADR-4):
impl Runtime { pub fn liveness(&self) -> HealthReport; } // RuntimeInner internal check
```

## Testing Strategy

| Layer | What | Approach |
|-------|------|----------|
| Unit | `fold`: Required+Unhealthy⇒Unhealthy; Optional+Unhealthy⇒Degraded; order-independence; empty⇒Healthy | domain tests |
| Unit | `HealthContributor` object-safe (`Arc<dyn _>`); closed-set guarantee lives in the type — a type/API-contract check that the public failure surface is ONLY `Option<HealthCode>` over a CLOSED `HealthCode` enum with NO string-carrying variant (no `Other(String)`/`Unknown(String)`), NOT a synthetic "struct never adds a `String` field" compile-test | domain tests |
| Unit/Integration | Exhaustive failure→structured-code mapping (`ProviderHealthContributor` mapping tests) — every contributor/provider failure maps to a fixed `HealthCode` | domain/runtime tests |
| Unit | Liveness path takes no registry / consults zero contributors (structural) | service-sdk test |
| Integration | Slow contributor times out ⇒ `Timeout` code, others unaffected; global budget honored; concurrent (not sequential) | `#[tokio::test]` |
| Integration | `ProviderHealthContributor` maps `ProviderHealth::Unhealthy`⇒`DependencyFailure`; Required default | runtime test |
| Integration | not-yet-initialized ⇒ `InitializationPending` distinct from steady-state fail | service-sdk test |

## Threat Matrix

N/A — no routing, shell, subprocess, VCS/PR automation, executable-file
classification, or process-integration boundary. Data-exposure risk (leaking
internal detail) is structurally closed by the closed `HealthCode` set (no
free-text field), not the process-integration matrix.

## Migration / Rollout

Purely additive at the model layer; nothing wired to a transport in this change.
`ProviderHealth` SPI unchanged. The only removal is the unconsumed parallel
provider readiness surface. Rollback = drop the health module + registry wiring
and restore `readiness()`/`ProviderSubsystemReadiness`. No schema/migration.

## Open Questions

None blocking. `HealthCode` variants are a design proposal (the spec left the
set open) and may be refined during apply without changing the closed-set
guarantee.
