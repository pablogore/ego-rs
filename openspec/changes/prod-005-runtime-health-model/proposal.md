# Proposal: PROD-005 — Runtime Health Model

## Intent

`ROADMAP.md:651` frames PROD-005 ("Health, Readiness and Startup") HTTP-first, leading with `/live` `/ready` `/startup` endpoints. This is the wrong framing for a transport-agnostic framework: `ego-transport`'s own lib.rs already declares HTTP "mechanism only" (AD-2), and PR #234/#235 landed a provider-scoped `ProviderHealth` model with **no free-text** and an explicit liveness/readiness distinction. Today there is **no framework-level** health/readiness/startup model — only the provider subsystem has one, and there is deliberately no process/service readiness surface (`access.rs:341` doc). PROD-005 renames the change to **Runtime Health Model** and defines a transport-agnostic health/readiness/startup MODEL in Ego's core. Transport (HTTP/gRPC/GraphQL/broker/CLI/TUI) is left entirely to adapters; no transport is privileged.

## Scope

### In Scope
- Liveness, readiness, and startup/initialization semantics as framework concepts.
- `HealthContributor` contract; `Required` vs `Optional` dependency requirement.
- Richer status `{Healthy, Degraded, Unhealthy}`; safe closed `HealthCode` set (no free text).
- Concurrent aggregation: fan-out, per-contributor timeout, optional global budget, deterministic aggregation rules.
- Lifecycle registration of contributors; async execution in service-sdk/runtime.
- TestKit support (same-contract real production types).
- Integrating the #234/#235 provider subsystem as a contributor to the single model.

### Out of Scope (Non-Goals / Follow-ups)
- `/live` `/ready` `/startup` endpoints; Kubernetes probe wiring.
- gRPC health service; GraphQL schema; broker subjects.
- Dashboards, alerting, infrastructure-specific health implementations.

## Frozen Decisions (decided constraints, not open questions)

1. **Liveness strictly separated from readiness.** Liveness = minimal internal health of the process/runtime; it **MUST NOT** consult external dependencies. A DB/broker/provider/remote-auth failure **MUST NOT** be able to fail liveness. Readiness **DOES** aggregate external contributors. (k8s: liveness failure restarts the pod; readiness failure only removes it from rotation — conflation turns a dependency blip into a restart storm.)
2. **No public free-text.** Internal detail MAY exist for logs/traces, but the public contract **MUST** use a structured, closed code set, e.g. `HealthCode { Timeout, Unavailable, InitializationPending, DependencyFailure, InternalFailure }`. Preserves the `ProviderHealth` philosophy; never expose backend messages.
3. **Richer status than binary:** `HealthStatus { Healthy, Degraded, Unhealthy }`, plus explicit `DependencyRequirement { Required, Optional }`. The aggregator computes the global result from `(status, requirement)`: an Optional+Unhealthy contributor **MUST NOT** force global readiness false, but **SHOULD** surface as `Degraded`.
4. **Concurrent aggregation.** The contract **MUST** require concurrent fan-out + per-contributor timeout + optional global budget; no single contributor can block the rest. A timeout becomes a structured state (`HealthCode::Timeout`), never a hung aggregator. The existing sequential polling (`access.rs:341`) **MUST NOT** be copied.
5. **Layering.** `ego-domain`: `ProbeKind`, `HealthStatus`, `HealthCode`, `DependencyRequirement`, the `HealthContributor` contract, report/value types. service-sdk/runtime: registry, async execution, timeout, concurrent aggregation, lifecycle integration. Adapters (out of scope): HTTP/gRPC/GraphQL/broker/CLI-TUI/custom.
6. **Transport neutrality (NORMATIVE):** "No health capability may require or privilege HTTP, gRPC, GraphQL, messaging, Kubernetes, or any other transport/deployment mechanism." and "Adapters consume the same framework-level health report and map it to protocol-specific representations."
7. **#234/#235 compatibility.** Goal is a SINGLE model, not two parallel ones: the provider subsystem becomes a contributor to the global model. Design must choose among adapt existing types / deprecate / keep as subsystem-internal — intent is to avoid duplicated semantics.

## Open Fork for DESIGN (do not resolve here)

`HealthContributor` in `ego-domain` shape: **(A)** async/object-safe trait in domain, **(B)** pure synchronous contract with the async runner outside domain, or **(C)** object-safe port returning a boxed future. Concern: not contaminating `ego-domain` (zero infra deps) with a premature runtime/future decision. The design **MUST** decide consciously.

## Capabilities

### New Capabilities
- `runtime-health-model`: transport-agnostic liveness/readiness/startup model — domain contracts (`ProbeKind`, `HealthStatus`, `HealthCode`, `DependencyRequirement`, `HealthContributor`, report types) + service-sdk/runtime registry, concurrent aggregation, timeout, lifecycle integration, TestKit support.

### Modified Capabilities
- `external-data-providers`: provider subsystem (`ProviderHealth`, `ProviderSubsystemReadiness`, `RuntimeDataProviderAccess::readiness()`) becomes a contributor to the unified model; exact adapt/deprecate mechanism decided in design.
- `service-sdk`: contributor registration integrated with lifecycle (`LifecycleManaged`), which today has no health/readiness method.

## Approach

Define the value types and `HealthContributor` contract in `ego-domain` (no infra). Place the registry, concurrent async runner (fan-out + per-contributor timeout + optional global budget), and lifecycle wiring in service-sdk/runtime. Aggregation is deterministic: liveness never consults contributors; readiness folds `(status, requirement)` per contributor into a global report. Adapters map the same report to their protocol. The provider subsystem is refactored to feed the one model rather than expose a parallel one.

## Affected Areas

| Area | Impact | Description |
|------|--------|-------------|
| `crates/domain/src/` (new health module) | New | `ProbeKind`, `HealthStatus`, `HealthCode`, `DependencyRequirement`, `HealthContributor`, report types |
| `crates/service-sdk/src/` (registry + lifecycle) | New/Modified | Registry, concurrent aggregation, timeout, lifecycle registration |
| `crates/runtime/src/providers/` | Modified | Provider subsystem becomes a contributor (types possibly adapted/deprecated) |
| `crates/testkit/src/` | New | Same-contract health test building blocks |

## Risks

| Risk | Likelihood | Mitigation |
|------|------------|------------|
| Async/future decision contaminates `ego-domain` | High | Deferred to design as an explicit fork (A/B/C); domain holds only contracts/values |
| Duplicated semantics vs #234/#235 provider model | Med | Single-model mandate; provider subsystem becomes a contributor |
| Liveness accidentally consults dependencies | Med | Normative hard rule; contract forbids external calls on the liveness path; TestKit coverage |
| Sequential polling reused, causing a hung aggregator | Med | Contract mandates concurrent fan-out + per-contributor timeout |
| Transport creeps into core | Low | Normative transport-neutrality clause; adapters out of scope |

## Rollback Plan

Purely additive at the model layer; nothing is wired to a transport in this change. If unwanted, drop the new health module and registry wiring and revert the provider-subsystem contributor refactor — `ProviderHealth`/`ProviderSubsystemReadiness`/`readiness()` return to their #234/#235 behavior. No runtime behavior depends on the new model until an adapter (future change) consumes it, so revert is behavior-neutral. No schema/migration impact.

## Dependencies

- Builds on PR #234 (hardened by #235): `ProviderHealth`, `ProviderSubsystemReadiness`, `RuntimeDataProviderAccess::readiness()`.
- `LifecycleManaged` (`service-sdk/src/implementation.rs:42`) as the lifecycle seam.

## Success Criteria

- [ ] Domain exposes `ProbeKind`, `HealthStatus`, `HealthCode`, `DependencyRequirement`, `HealthContributor`, report types — with zero infra/transport deps.
- [ ] Liveness path provably cannot consult external contributors.
- [ ] Readiness aggregates contributors concurrently with per-contributor timeout; Optional+Unhealthy yields Degraded, not global failure.
- [ ] `HealthCode` is a closed set; no free-text leaves the public contract.
- [ ] Provider subsystem participates as a contributor to the single model (no parallel model).
- [ ] TestKit exposes same-contract building blocks; `cargo test --workspace` green.
