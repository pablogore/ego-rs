# Proposal: CORE-019A — External Data Providers SPI

## Metadata

| Field | Value |
|-------|-------|
| Change ID | CORE-019A |
| Title | External Data Providers SPI |
| Type | Core change (read-side external data access) |
| Date | 2026-07-15 |
| Parent | — (roadmap follow-up named by CORE-019 §14, `openspec/changes/archive/2026-07-15-core-019-reliable-external-effects/proposal.md`) |
| Related | CORE-019 (archived, write-side counterpart — no technical dependency); CORE-011A (`KeyResolver`, the existing particular case this SPI generalizes) |
| Status | PROPOSING |

## 1. Intent — Primary Architectural Objective

**The primary decision this change makes is establishing the ownership
boundary for external read-side integrations while preserving ego-rs's
dependency layering (`domain` → `persistent-entity`/`runtime` →
`service-sdk`).** Everything else in this proposal — trait names, resolution
models, contract shapes — serves that one decision.

CORE-019 settled the write side: effects are described by the domain, owned
by the runtime, executed behind a registry. The read side has no equivalent
sanctioned home. A handler that needs external data today has exactly one
in-repo precedent — `KeyResolver` (`crates/security-jwt/src/key_resolver.rs`)
— a one-off port living in a security crate, invisible to the runtime,
unregistered, uninstrumented, and unowned by any lifecycle. Every future
external-read need (reference data, third-party lookups, feeds) would either
copy that ad-hoc shape or invent a new one, eroding the layering CORE-019
just paid to protect.

### Why This Is a Core Capability

- External data needs recur across domains (reference data, third-party
  lookups, feeds) — a single sanctioned capability avoids each domain
  reinventing its own ad-hoc port, as `KeyResolver` already had to.
- Registration, discovery, and observability must be consistent across the
  whole running service, not decided independently per handler or per
  domain.
- Ad-hoc, per-domain implementations fragment the architecture and erode
  the layering CORE-019 just established.

## 2. Current Gap (verified against source)

1. **CORE-019A is already named and scoped, not invented here.**
   `openspec/specs/external-effects/spec.md` Non-Goals and CORE-019 proposal
   §14 / Decision Summary #10 explicitly defer "read-side External Data
   Providers" to CORE-019A — related, sequenced after, no technical
   dependency, not designed there.
2. **The general pattern exists only as one specific instance.**
   `KeyResolver` is a *particular case* of the general `ExternalDataProvider`
   pattern — async trait, `Send + Sync`, object-safe, cache-first semantics
   (AD-013) — hard-wired to one domain (JWT keys) in one crate. CORE-019A
   generalizes something that already exists in narrower form; it does not
   invent an unrelated abstraction. The direction is
   `ExternalDataProvider` (general) ← `KeyResolver` (specific existing
   instance), never the reverse.
3. **No `DataProvider`, SPI, or plugin trait exists anywhere in the
   workspace.** Genuinely greenfield surface; no migration burden.
4. **Naming is already fixed by CORE-019 §14**: *External Data Providers*,
   matching the dominant `*Provider` suffix (`AuthenticationProvider`,
   `ConfigurationProvider`, CORE-014 authorization providers) and keeping
   distance from the write-side `Effect` vocabulary.
5. **Designing the SPI before selecting a production adapter is
   intentional.** CORE-019 introduced its execution contracts
   (`ExternalEffectExecutor`, retry policy) before any concrete transport
   existed; CORE-019A follows the same order — contract first, adapters
   later (§9 Non-Goals, §16 risk).

## 3. Architectural Principles

CORE-019A follows the same architectural principles CORE-019 already
established for external-integration surfaces in this codebase. These are
constraints on what the design phase may choose, not the choices themselves:

- **Explicit ownership** — every provider has exactly one registered owner;
  nothing is implicit.
- **Fail closed** — an unregistered lookup or a duplicate registration is a
  loud error, never a silent default.
- **Zero runtime cost when unused** — no provider registered means no
  measurable overhead.
- **No reflection** — providers are discovered through explicit
  registration, never runtime scanning.
- **Explicit registration** — registration is an explicit call
  (`RuntimeBuilder`), not implicit or auto-discovered.
- **Explicit lifecycle ownership** — startup and shutdown have one clear
  owner, not left to each provider (which component holds that owner role
  is AD-001, undecided here).
- **Transport-unaware SPI** — the contract never leaks HTTP/gRPC/Redis/etc.
  -specific shapes into the runtime or handler layer.
- **Dependency layering preserved** — no new edge violates
  `domain` → `persistent-entity`/`runtime` → `service-sdk`.
- **Testable by construction** — providers must be replaceable by
  deterministic test doubles in `testkit`, continuing the CORE-017/018/019
  TestKit-first convention.

## 4. Proposed Capability — Two Concepts, Not One

Mirroring CORE-019's own `ExternalEffectExecutor`/`ExecutorRegistry` split,
this proposal introduces **two distinct responsibilities as two distinct
concepts**, never conflated:

1. **`ExternalDataProvider`** — the fetch capability itself. One provider
   knows how to obtain one kind of external data. It is single-purpose and
   protocol-owning (the runtime never learns transports), exactly as an
   `ExternalEffectExecutor` owns one attempt of one effect.
2. **`ExternalDataProviderRegistry`** — lookup and ownership of providers.
   Keyed registration, one owner per key, duplicate registration fails at
   registration time (the CORE-019 fail-closed registry precedent), wired
   through `RuntimeBuilder` in `service-sdk`.

The split matters because the two evolve independently: the provider contract
is the public SPI applications implement; the registry is the runtime's
ownership/discovery mechanism. CORE-019's design discipline of classifying
every type as Public SPI / Internal runtime / Private helper up front applies
here from day one (design-phase deliverable).

## 5. Resolution Models — Open Design Alternatives

**Discovery and resolution are distinct concerns, kept separate on
purpose.** Discovery — how a provider becomes known to the runtime, at
startup, through explicit registration — is fixed by §4/§9: keyed
registration through `RuntimeBuilder`, duplicates fail at registration time.
Resolution — how a *handler*, per request, reaches the specific provider it
needs — is **not** fixed, and is the subject of this section.

Three resolution models go to design as live alternatives:

| Model | Shape | Proposal-level tradeoffs |
|-------|-------|--------------------------|
| A | handler → `provider.fetch(...)` directly (provider injected into the entity/handler struct) | Simplest call path, no new plumbing; but bypasses any central ownership/instrumentation chokepoint, each handler wires its own dependency, and the registry becomes optional — weakening the ownership boundary this change exists to establish |
| B | handler → registry → provider | Central ownership, one instrumentation/lookup chokepoint, fail-closed on missing provider; but the handler needs a registry handle, raising the question of which crate exposes that handle without a new cross-layer edge (CORE-019's AD-3/AD-4 port-location split is the precedent to follow) |
| C | handler → `CommandContext` → registry → provider | Zero new handler fields — `handle_command(&self, command, state, context)` already carries a `CommandContext`; but couples `CommandContext` to a runtime-owned concern, a layering question in its own right, and widens a context type every handler sees |

**Selection criterion**: the chosen model should minimize new dependency
edges while preserving explicit ownership and observability (§3
principles) — this is the objective test the design AD should argue
against, not a stylistic preference.

Design must pick one (or a justified hybrid) and record it as an AD with the
rejected alternatives, per this project's design rules.

## 6. Open Contract Questions (design-phase, flagged now)

1. **Fetch contract shape** — `fetch(key)` vs `fetch<T>(...)` vs
   `query(...)` vs `execute(...)` vs a Request object. Each implies a
   different genericity/object-safety tradeoff (`Arc<dyn …>` registries
   need object safety; typed results fight it). **Open — design decides.**
2. **Sync vs async and the cache-first constraint** — precedent
   (`KeyResolver`, `ExternalEffectExecutor`) is async trait + `Send + Sync`.
   If providers must be callable from a synchronous command path, the
   AD-013 cache-first contract (`KeyResolver`'s bridge via
   `futures_executor::block_on`) must be adopted explicitly, not assumed.
3. **Lifecycle** — singleton vs scoped vs stateless providers; who owns
   caches; runtime lifetime; registration ordering, startup cost when
   unused (zero-cost precedent from CORE-019 §10), and shutdown behavior —
   mirroring how CORE-019 resolved registration/ownership/drain/wiring.
   Providers that hold long-lived resources (an HTTP connection pool, a
   gRPC channel, a Redis/NATS client, an S3 client) have a materially
   different lifecycle than stateless fetch functions; whether the SPI must
   accommodate long-lived provider state, and who owns its teardown, is an
   explicit open question, not assumed away.
4. **Error/retry model** — reuse CORE-019's `RetryPolicy`/backoff
   (`crates/runtime/src/effects/policy.rs`) or the read-side
   `ProjectionError::{Transient, Fatal, PoisonEvent}` taxonomy
   (`crates/domain/src/read_side/error.rs`). Two existing conventions;
   CORE-019A must pick one deliberately, never invent a third.
5. **Registry key / provider identifier** — what identifies a provider in
   the registry: a `String` key, `TypeId`, trait-object identity, the
   request type itself, or a dedicated provider-id type. Each has different
   ergonomics/collision-safety tradeoffs (typed keys prevent stringly-typed
   collisions; string keys are simpler to log and configure). **Open —
   design decides.**
6. **Crate placement** — a parallel shape to CORE-019 is plausible
   (handler-facing trait near `persistent-entity`/`domain`, registry and
   ownership in `runtime`, wiring in `service-sdk`), but CORE-019 explicitly
   declined to design this. **Open — design decides.**

## 7. Candidate Functional Requirements

- **Observability is likely a functional requirement, not an afterthought**:
  per-fetch latency, timeouts, retries, cache hit/miss, and provider name as
  correlation fields, aligned with CORE-012A infrastructure and CORE-019's
  signal precedent (payloads/sensitive values never logged by default).
  Providers must integrate with the runtime's existing observability
  pipeline rather than emitting telemetry independently.
- **Fail-closed resolution**: requesting a provider key with no registration
  should fail loudly (CORE-019 `ExecutorMissing` precedent), never return a
  silent default.
- **Tenant scoping**: providers likely receive the established tenant as a
  fact they cannot mint (CORE-008A precedent), if fetches are tenant-scoped.
- Exact signal list and requirement wording belong to the spec phase.

## 8. Relationship to KeyResolver (CORE-011A)

`KeyResolver` is a **particular case** of the general `ExternalDataProvider`
pattern: it already has the shape (async, `Send + Sync`, object-safe,
cache-first per AD-013) but is domain-specific and privately owned by
`security-jwt`. CORE-019A generalizes that proven shape into a shared, runtime-integrated
SPI (exact ownership per AD-001, §11). **Retrofitting `KeyResolver` onto the new SPI is out of scope** — it is
a candidate follow-up once the SPI exists, and forcing it now would couple
this change to the security stack for no first-slice benefit.

## 9. Scope

### In Scope

- The `ExternalDataProvider` public SPI contract (shape open per §6).
- The `ExternalDataProviderRegistry` ownership contract: keyed, one owner
  per key, duplicate registration fails at registration time.
- Registration wiring through `RuntimeBuilder` (`service-sdk`).
- Lifecycle integration: startup, zero cost when unused, shutdown.
- Observability signals (candidate FR, §7).
- Extension-surface classification (Public SPI / Internal / Private) in design.
- Test-double support in `testkit` and one trivial provider dogfooded in
  `examples/reference-app` (per CORE-019/CORE-018 convention).
- New canonical spec `openspec/specs/external-data-providers/`.

### Out of Scope / Non-Goals

- Concrete adapters for HTTP, databases, gRPC, or any real external system
  (tests/reference-app may use trivial local providers).
- Retrofitting `KeyResolver` or any existing `*Provider` onto the SPI (§8).
- Write-side delivery, effects, idempotent dispatch — all CORE-019, shipped.
- Durable/shared cache store; distributed cache coordination.
- Circuit breaker (same deferral rationale as CORE-019 §7).
- Background refresh/scheduling infrastructure beyond what lifecycle wiring
  requires.

## 10. Capabilities

### New Capabilities

- `external-data-providers`: read-side external data provider SPI, registry
  ownership, resolution, lifecycle, and observability contracts.

### Modified Capabilities

- `service-sdk`: `RuntimeBuilder` gains provider registration; provider
  startup/shutdown is integrated into the existing lifecycle (exact
  ownership per AD-001, §11).
- `persistent-entity`: **conditionally modified** — only if design selects
  resolution model B or C (§5) does the handler-facing surface (context or
  port trait) change; model A leaves it untouched. The spec phase must treat
  this delta as contingent on the design AD.

## 11. Design Decisions the Design Phase Must Answer (AD-001–AD-012)

Proposal-level framing; each maps to one design AD, none resolved here.

| AD | Decision | Proposal-level framing |
|----|----------|------------------------|
| AD-001 | Ownership | The proposal intentionally leaves the owning crate undecided (§1). Design evaluates alternative placements against dependency layering — this is not assumed to be settled. |
| AD-002 | Registry | Discovery — how a provider becomes known at startup through registration (§4/§9) — distinct from resolution (AD-003). |
| AD-003 | Resolution | Models A/B/C (§5) and the selection criterion (minimize new dependency edges, preserve ownership + observability) — the central per-request decision, including whether/how `CommandContext` is involved (model C). |
| AD-004 | Provider Contract | `fetch(key)` vs `fetch<T>(...)` vs `query(...)` vs `execute(...)` vs a Request object (§6.1). |
| AD-005 | Registry Key | What identifies a provider: `String`, `TypeId`, trait-object identity, request type, or dedicated provider-id type (§6.5). |
| AD-006 | Lifecycle | Singleton/scoped/stateless, cache ownership, zero-cost-when-unused, shutdown, and long-lived provider state/teardown (§6.3). |
| AD-007 | Error Model | CORE-019's `RetryPolicy` vs the read-side `ProjectionError` taxonomy — pick one deliberately (§6.4). |
| AD-008 | Observability | Latency, timeouts, retries, cache hit/miss, provider name, integrated with the runtime's existing observability pipeline (§7). |
| AD-009 | Crate Placement | Handler-facing trait vs registry/ownership crate split (§6.6). |
| AD-010 | Testability | Providers must be replaceable by deterministic test doubles in `testkit`, continuing the CORE-017/018/019 TestKit-first convention (§3, §9). |
| AD-011 | KeyResolver Relationship | General ← specific (§8); `KeyResolver` is prior art, retrofit out of scope. |
| AD-012 | Genericity | How abstract the SPI contract should be — the guardrail against the over-general-SPI risk (§13): new generic parameters or features require a second real consumer, not speculation. |

## 12. Affected Areas

| Area | Impact | Description |
|------|--------|-------------|
| `crates/runtime` (new module) | New | Provider SPI and/or registry, policies (placement per design) |
| `crates/persistent-entity` | Conditional | Handler-facing surface only if model B/C is chosen (§5, §11 AD-003) |
| `crates/service-sdk/src/runtime/builder.rs` | Modified | Provider registration, lifecycle ordering |
| `crates/testkit` | Modified | Provider test doubles |
| `examples/reference-app` | Modified | One trivial provider dogfooded |
| `openspec/specs/external-data-providers/` | New | First canonical spec for this capability |
| `crates/security-jwt/src/key_resolver.rs` | Referenced | Prior art only; unchanged |

## 13. Risks

| Risk | Likelihood | Mitigation |
|------|------------|------------|
| No concrete in-repo use case yet — SPI shaped in the abstract may not fit the first real provider | Med | Dogfood one trivial provider in reference-app as forcing function; keep contract minimal; treat §6.1 shape as revisitable |
| Resolution-model choice (A/B/C) leaks runtime concerns into domain/handler layering | Med | §5 tradeoffs recorded; design AD must show the dependency edges, per no-circular-deps rule |
| Over-generalizing from one data point (`KeyResolver`) | Med | SPI generalizes the *shape* only; retrofit deferred until a second real consumer validates it |
| Third error-handling convention emerges | Low | §6.4 constrains design to one of the two existing conventions |
| Scope creep toward caching infrastructure/circuit breakers | Med | Non-goals §9; breaker deferral mirrors CORE-019 |
| Over-general SPI — designing for hypothetical providers rather than a real one, distinct from simply lacking a use case | Med | Dogfood one trivial provider in reference-app before stabilizing the contract; new generic parameters/features require a second real consumer, not speculation (AD-012 Genericity, §11) |

## 14. Rollback Plan

All new contracts live in new modules behind opt-in registration. Rollback =
remove provider registration from `RuntimeBuilder` and delete the new
modules; no existing handler, frozen domain type, or `KeyResolver` call site
is touched, so revert is a clean commit-range revert with no data migration.

## 15. Dependencies

- CORE-019 (archived, shipped) — vocabulary and registry/lifecycle
  precedent only; **no technical dependency** (per CORE-019 §14).
- CORE-017 lifecycle, CORE-008A tenant contracts, CORE-012A observability —
  reused as-is where design adopts them.
- No new external crates anticipated beyond the existing async stack.

## 16. Success Criteria

- [ ] A reference-app handler obtains external data through a registered
      provider via the designed resolution path — never by constructing a
      client inline.
- [ ] Duplicate provider registration for the same key fails at
      registration time.
- [ ] Resolving an unregistered provider key fails loudly, never silently.
- [ ] Zero measurable runtime overhead when no provider is registered.
- [ ] Existing `PersistentEntity` implementations compile unchanged.
- [ ] Every AD-001–AD-012 decision (§11) is answered by a recorded design AD
      with rejected alternatives.
- [ ] Fetch operations emit the §7 observability signals.
- [ ] No provider implementation requires knowledge of runtime internals.

## 17. Open Questions for Design

1. Resolution model A/B/C (§5) — the central decision.
2. Fetch contract shape (§6.1).
3. Sync-bridge/cache-first adoption of AD-013 (§6.2).
4. Provider lifecycle, cache ownership, and long-lived provider state/teardown (§6.3).
5. Error/retry convention selection (§6.4).
6. Registry key / provider identifier (§6.5).
7. Crate placement of trait vs registry (§6.6).
8. Concrete first domain use case — nothing in-repo names one; the trivial
   reference-app provider stands in until a real consumer exists (product
   input welcome before or during design).
