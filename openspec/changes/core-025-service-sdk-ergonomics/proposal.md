# Proposal: CORE-025 — Service SDK Developer Ergonomics

Reduce the accidental cost of the Service SDK developer journey (define → contract → dependencies → build → register → typed reference → invoke → test → configure → understand errors). No new capabilities, no runtime redesign. Evidence base: `explore.md` (Phase 1 Ergonomics Audit, this folder) — cited throughout, not re-derived.

## Problema verificado

Three audit findings, all evidence-backed (explore.md, Section B):

- **F-01 (CRITICAL)**: the SDK's documented service-registration path does not exist. `Runtime` has no resolution method, `RuntimeBuilder` has no service-registration method, and `ServiceRegistry`/`Resolvable` are called only from their own unit tests. Every real test and example hand-rolls a 4-step proxy construction instead.
- **F-02 (CRITICAL)**: `RuntimeBuilder::build()` performs zero dependency validation ("Always succeeds", doc comment) — a missing adapter/config surfaces only when `Injectable::build()` runs, i.e. at first invocation in a hypothetical production bootstrap, not at startup. `Injectable::dependencies()` is compile-time-complete but never consulted.
- **F-03 (HIGH)**: `RuntimeError::DependencyNotFound` is a bare unit variant — no `Display`, no `std::error::Error`, no type name, no service name. Debug output is literally `DependencyNotFound`.

This is not "streamline a working baseline" — the audit's core reframe is that the intended path was **built but never wired**; the fix is completion of an existing designed mechanism, not invention (explore.md, Executive Summary and Section D).

## Evidencia del repositorio

All citations from explore.md:

| Claim | Location |
|---|---|
| Registry field permanently empty, `#[allow(dead_code)]` | `runtime/runtime_builder.rs:115-117` |
| Doc comments advertise a resolution call that does not compile | `runtime/resolvable.rs:4,42` |
| `ServiceRegistry::register`/`resolve_raw` — zero non-test call sites | `registry/registry.rs:78-121` |
| Hand-rolled 4-line proxy ceremony | `tests/proxy_codegen.rs:75-81`; duplicated in ≥4 files (F-06) |
| `build()` "Always succeeds", no validation | `runtime/builder.rs` doc comment |
| `DependencyNotFound` bare variant, no `Display` | `runtime/runtime_builder.rs:398-403` |
| TestKit "forward compatibility with a future public resolve" | `crates/testkit/src/fixtures.rs:82-83` |
| `ServiceFactory` has zero impls repo-wide | `implementation.rs:56-63` (F-04) |

## Experiencia actual

From explore.md Section A: a developer defines a trait with `#[service]`/`#[operation]`, implements it, then — because no registration API works — hand-assembles `RuntimeBuilder::new().build()`, `Arc::downgrade(rt.inner())`, an empty `InterceptorChain`, and `{Trait}Ref::new(inner, chain, weak)`. This ceremony is duplicated ≥4 times across test files (F-06). TestKit has no helper for enforcement-wrapped trait proxies (F-07). A forgotten dependency fails late with an error naming nothing. The documented path (`ServiceRegistry`/`Resolvable`) does not compile if attempted.

## Experiencia objetivo

Outcomes, not API shapes (shape is Design's job — OQ-1):

- **One registration call** per service against the builder, wiring the already-implemented `ServiceRegistry`/`Resolvable` machinery into the public surface.
- **One resolution call** yielding the macro-generated typed `{Trait}Ref`, with the interceptor chain, weak runtime handle, and enforcement wiring assembled internally — nothing hidden, just no longer hand-duplicated.
- **Fail-fast bootstrap**: missing dependencies detected when the runtime is built (walking `Injectable::dependencies()` against provided adapters/configs), not at first invocation. Compile-time detection is out of reach in principle (F-11) — "fail at build, not at first call" is the ambition.
- **Errors that name things**: the missing type and the requesting service, via `Display`. Reporting *every* missing dependency at once (F-08) is deferred to a follow-up change — this slice fixes what the error says, not how many it reports per attempt.
- **TestKit uses the same canonical path** as production — no parallel wiring.

The exact registration/resolution API shape — method names, signatures, what gets registered, how versioning interacts with `ContractVersion` — is **explicitly deferred to design.md per OQ-1** (explore.md, Open Questions). Every mention here means "a canonical registration/resolution API", nothing more specific.

## Alcance

| Finding | In this slice? | Rationale |
|---|---|---|
| F-01 wire registration/resolution | **Yes** | The core gap; everything else attaches to it |
| F-02 fail-fast build validation | **Yes** | Depends on F-01; audit Q12 names F-01+F-02/F-03 as the minimum perceptible slice |
| F-03 diagnostic error payload + `Display` | **Yes** | Low-risk, additive; pairs with F-02 |
| F-06/F-07 TestKit proxy helper | **Yes** | Required by Scenario 4 (same canonical path); natural byproduct of F-01 |
| F-09 minimal end-to-end example | **Yes (thin, last)** | Doubles as acceptance evidence that the happy path exists |
| F-04 `ServiceFactory` deletion | **Deferred** | Independent small cleanup, CORE-008B playbook — own micro-change |
| F-05 COOKBOOK.md rewrite | **Deferred** | Must sequence AFTER code lands (audit F-05: otherwise goes stale immediately) |
| F-08 aggregate missing-dep errors | **Deferred** | F-02 (build validation) + F-03 (diagnostic `Display`) already change observable behavior twice; "report every missing dep at once" is a distinct improvement, not required to make F-02/F-03 useful. Own follow-up (CORE-025b or micro-change) so this slice answers only "is there a canonical path?" and "are errors useful?" — not also "how do we aggregate multiple errors?" |
| F-10 file naming | **Deferred** | Cosmetic, mention only (audit's own call) |

### Escenarios que spec/tasks deben cubrir (detail in spec.md, not here)

1. Minimal service (no deps) — defined, built, registered, invoked, tested.
2. Service with dependencies (adapter + typed config, same existing DI mechanism).
3. Missing dependency — what error the developer gets; detectable at build/bootstrap time instead of first invocation.
4. TestKit — service built through the same canonical path as production, no parallel wiring.
5. Protected service — the ergonomic improvement must not bypass `ServiceContext`, authorization, tenant enforcement, interceptors, or contract registration.

### Principios obligatorios (how scope honors each)

1. **Explicit Rust over hidden magic** — registration and resolution are explicit calls; nothing auto-discovers services.
2. **Compile-time errors where possible** — typed tags/refs stay compile-checked; where compile-time is impossible in principle (F-11), fail at `build()`.
3. **Fail-closed security/tenant** — resolution goes through the same generated proxy running the existing guard order (authorize → tenant → interceptors → body); Scenario 5 is a hard gate.
4. **No ambient state / task-local context** — `ServiceContext` stays an explicit parameter; nothing changes here.
5. **No global service locator** — the registry lives inside the runtime instance a caller already holds; no statics.
6. **No duplicate DI container** — F-01 wires the *existing* `ServiceRegistry`/`Resolvable`/`Injectable`; nothing new is built beside them.
7. **Same path in production and TestKit** — F-06/F-07 helper delegates to the identical canonical path (the `FixtureBuilder` precedent, explore.md Section D).
8. **Macros reduce repetition, never hide lifecycle/permissions/deps** — no new macro semantics in this slice (F-08's codegen aggregation is deferred, see Alcance).
9. **No breaking object safety / public contracts without justification** — additive surface; existing `{Trait}Ref::new(...)` keeps working (see Compatibilidad).
10. **No abstraction to save two lines** — the slice removes a duplicated 4-step ceremony and a silent failure mode, not lines for their own sake.
11. **Preserve observability/diagnosability** — F-03 strictly improves it; guard ordering and interceptors untouched.
12. **Idiomatic Rust** — consuming-builder shape already in the codebase (`RuntimeBuilder`, `FixtureBuilder`) is the model; no framework mimicry.

## Fuera de alcance

No full Application Builder. No CLI. No scaffolding. No HTTP/gRPC/GraphQL transports. No scheduler work. No observability-platform work. No hot reload. No plugin system. No new runtime capabilities. This change is Service SDK ergonomics only. Plus the deferred findings above (F-04, F-05, F-10).

## Capacidades afectadas

### New Capabilities
- None — this completes existing `service-sdk` capability surface.

### Modified Capabilities
- `service-sdk`: the existing requirement "RuntimeBuilder::build() Behavior Is Unchanged" (openspec/specs/service-sdk/spec.md:340, from CORE-018b) is narrowly scoped to logger wiring, teardown ordering, and security-provider installation for correctly-built runtimes — it does **not** cover service-dependency validation and is **not** superseded by F-02 (verified by reading the full requirement: none of its 4 scenarios touch adapters/config/dependencies). F-02 needs a *new*, separate requirement for fail-fast dependency validation on service registration; the CORE-018b requirement stays as-is, unmodified. New requirements also needed for canonical registration/resolution and diagnosable dependency errors. "RuntimeInner Not Publicly Constructible", guard-order, and tenant/authz requirements must be preserved unchanged (Scenario 5).
- `testkit`: new requirement for constructing enforcement-wrapped trait proxies through the canonical path (F-06/F-07).

## Blast radius

| Area | Impact |
|---|---|
| `crates/service-sdk/src/runtime/{builder.rs, runtime_builder.rs, resolvable.rs}` | Modified — registration/resolution wiring, build validation, error payload |
| `crates/service-sdk/src/registry/` | Wired into public surface (its existing contract must not be contradicted — OQ-1) |
| Generated macro API (`Resolvable` impl shape, `{Trait}Tag`/`{Trait}Ref`, emitted by `crates/service-sdk-macros/src/lib.rs`) | **Affected regardless of F-08.** Whatever OQ-1 decides for the registration/resolution contract, `Resolvable` is macro-generated per service — the macro's generated-code contract changes even though no user source code does. Design must treat this as a real compatibility surface (existing generated impls, `golden_codegen.rs`/`proxy_codegen.rs` snapshot tests), not an internal implementation detail. |
| `crates/testkit/src/fixtures.rs` | Modified — proxy helper (F-06/F-07) |
| `examples/` | New minimal end-to-end example (F-09) |
| `COOKBOOK.md` | Later, separate change (F-05 deferred) |

## Riesgos

| Risk | Likelihood | Mitigation |
|---|---|---|
| Careless change to `RuntimeBuilder` public surface breaks callers | Med | Additive-only surface; existing requirement deltas made explicit in spec |
| Wrong registration API shape is expensive to fix later | Med | Exactly why OQ-1 goes to design.md with explicit alternative comparison — not decided here |
| Ergonomic path accidentally bypasses enforcement | Low | Scenario 5 as acceptance gate; resolution reuses the generated proxy verbatim |
| Slice exceeds review budget | Med | Tasks phase forecasts; F-04/F-05/F-08/F-10 already carved out |

## Compatibilidad

Existing hand-rolled `{Trait}Ref::new(inner, chain, weak)` construction keeps working — this change is additive, not a replacement, unless design.md decides otherwise with explicit justification. **The canonical path changes; the low-level `::new()` constructor remains supported as an escape hatch for advanced integrations and tests, unless explicitly deprecated in a future change with its own justification.** This is a deliberate, standing commitment, not an oversight to be "cleaned up" later just because a canonical path now exists — a future contributor citing "we have `resolve()` now" is not sufficient grounds on its own to remove `::new()`. Existing `with_adapter`/`with_config`/security/tenant builder knobs unchanged.

## Rollback

Straightforward: the surface is additive. Revert the change commits; hand-rolled construction (today's only working path) is untouched and remains the fallback. The one behavioral change — `build()` failing fast on missing deps — only triggers for services registered through the new API, so reverting removes both together.

## Criterios medibles de éxito

Against explore.md Section C baseline (minimal / DI / security columns):

- [ ] Registration steps that actually work: **0 → ≥1** (a working canonical call exists; exact count set by design).
- [ ] Manual proxy-construction steps: **4 → 0** on the canonical path (hand path still available).
- [ ] Explicit types a developer must name for the minimal journey: reduced from 4 (`Arc<dyn Trait>`, `Arc<InterceptorChain>`, `Weak<RuntimeInner>`, `{Trait}Ref`) — target set by design, measured the same way.
- [ ] Hand-written boilerplate LOC (excl. business logic): **~20-25 → measurably lower**, re-measured post-implementation with Section C's method.
- [ ] Prerequisite-concept list no longer includes "`InterceptorChain` even with nothing to intercept", "`Weak<RuntimeInner>`", or "documented but non-functional registry".
- [ ] Missing dependency detected at `build()` with an error naming the missing type and requesting service (aggregating all missing deps per attempt is F-08, deferred — out of this slice's success criteria).
- [ ] Production/TestKit divergence for the trait-proxy path: **absent-on-both-sides → same canonical path, verified by test** (the `FixtureBuilder` proof pattern).
- [ ] Scenario 5: full guard order verified unchanged through the new path.

## Dependencies

- explore.md (this folder) — complete.
- design.md must resolve OQ-1 before spec/tasks commit to any API surface details that depend on it.
