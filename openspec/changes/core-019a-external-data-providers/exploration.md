# Exploration: External Data Providers SPI (CORE-019A)

## Hypothesis verification — confirmed, not assumed

This is **CORE-019A**, already named and scoped in the CORE-019 proposal — not something inferred from architecture alone:

- `openspec/specs/external-effects/spec.md` Non-Goals: "Read-side external providers (CORE-019A — related, sequenced after, no technical dependency, not designed here)."
- `openspec/changes/archive/2026-07-15-core-019-reliable-external-effects/proposal.md` §14 "Relationship with CORE-019A": "CORE-019 is command/write-side delivery; CORE-019A is read-side, I/O-capable ports for handlers that need external data... Related, sequenced after CORE-019... no technical dependency; CORE-019A is not 'Enabled by' CORE-019 and this proposal does not design it."
- Decision Summary #10 (same file): "What remains for CORE-019A? Read-side External Data Providers."
- Metadata table records `Related: CORE-019A (External Data Providers, proposed follow-up)`.

CORE-019 is outbound effect delivery (write-side); CORE-019A is the inbound/read counterpart — a pluggable interface for fetching external data.

## Naming precedent already fixed by CORE-019 §14

> "CORE-019A is **External Data Providers**, matching the dominant `*Provider` suffix (`AuthenticationProvider`, `ConfigurationProvider`, CORE-014 authorization providers) and the CORE-011A async-`KeyResolver` shape it would generalize."

Confirmed in codebase:
- `crates/security-sdk/src/providers/{basic,rbac,deny_all,allow_all}/mod.rs` — `BasicAuthenticationProvider`, `RbacProvider`, `DenyAllAuthorizationProvider`, `AllowAllAuthorizationProvider` (CORE-014 precedent for the `*Provider` suffix).
- `crates/security-jwt/src/key_resolver.rs` — `KeyResolver` trait, `#[async_trait]`, `Send + Sync`, object-safe (`Arc<dyn KeyResolver>`), with a doc comment demanding cache-first semantics (AD-013, from CORE-011A) so a sync call site can bridge safely. This is the shape CORE-019A is expected to generalize.

No existing `DataProvider`, `SPI`, or plugin trait exists anywhere in the workspace yet — this is genuinely greenfield.

## Existing SPI/port conventions to follow (from CORE-019, just shipped)

- **Port location split**: handler-facing trait lives in the crate the actor/handler already depends on (`EffectAcceptor` in `persistent-entity`), while the state-owning port(s) live in `crates/runtime` (`EffectStateStore`, `EffectDedupStore`) — chosen to avoid a new cross-layer dependency edge (AD-3/AD-4 in design.md).
- **Registry pattern**: `ExternalEffectExecutor` keyed by `effect_type` string, one owner per key, duplicate registration fails at registration time, wired through `RuntimeBuilder` in `service-sdk`.
- **Error taxonomy precedent**: `crates/domain/src/read_side/error.rs` has `ProjectionError::{Transient, Fatal, PoisonEvent}` with a documented retry policy (max 3, 100ms base, 10s max) — CORE-019 reused this rather than reinventing; CORE-019A likely should too, or reuse CORE-019's own `RetryPolicy`/backoff shape (`crates/runtime/src/effects/policy.rs`).
- **Extension-surface discipline**: CORE-019's design.md classifies every new type as Public SPI / Internal runtime / Private helper up front (§2 table) — worth doing the same for CORE-019A from the start.

## Where CORE-019A would naturally live (open design-phase decision)

Given the CORE-019 precedent and layering (`domain` → `persistent-entity`/`runtime` → `service-sdk`), a parallel shape is plausible: a fetch/read port trait near `persistent-entity` or `domain` (handler-facing, mirroring `EffectAcceptor`), with implementation/registry ownership in `runtime`, and registration wiring in `service-sdk`'s `RuntimeBuilder`. The CORE-019 proposal explicitly declines to design this ("not designed here"), so crate placement is a genuinely open design question, not a fact stated as decided here.

## Open questions (genuinely unresolved, not answered anywhere in-repo)

1. **Concrete domain meaning**: what is an "external data provider" concretely in ego-rs's domain — pricing/market data feed, third-party lookup API, reference data cache? Nothing in-repo names a concrete use case yet.
2. **Read/fetch contract shape**: single `fetch(key) -> Result<T, Error>`, or a query/multi-key batch shape? Invoked during `handle_command` (would need to reach into `CommandContext`), or entirely outside the command path (background cache warm/refresh, per CORE-011A's AD-013 cache-first constraint)?
3. **Sync vs async**: precedent (`KeyResolver`, `ExternalEffectExecutor`) is async trait + `Send + Sync`. If this needs to be callable from inside a synchronous command-handling path (as `KeyResolver` does via `futures_executor::block_on` under AD-013's cache-first contract), that constraint needs to be decided explicitly, not assumed.
4. **Error/retry model**: reuse CORE-019's `RetryPolicy`/backoff (`crates/runtime/src/effects/policy.rs`) or the older read-side `ProjectionError` taxonomy (`crates/domain/src/read_side/error.rs`)? Two different existing conventions in the same codebase; CORE-019A should pick one deliberately rather than invent a third.
5. **Relationship to `CommandContext`**: `crates/persistent-entity/src/persistent_entity.rs`'s `handle_command(&self, command, state, context)` already takes a `CommandContext` — is external-data fetching meant to flow through that context, or is it a separate injected dependency on the entity/handler struct?

## Housekeeping discovery (flagged, not part of this SPI)

`openspec/changes/core-019-reliable-external-effects/` (no date prefix, no `state.yaml`) is a stray, un-archived duplicate of the now-archived `openspec/changes/archive/2026-07-15-core-019-reliable-external-effects/` (same `proposal.md`/`exploration.md`/`design.md`/`tasks.md`). Likely a leftover from before archival; worth cleaning up separately so it doesn't confuse a future SDD phase into thinking CORE-019 is still active.

## Recommendation

Proceed to `sdd-propose` for CORE-019A: an async `*Provider`-suffixed port (following `KeyResolver`/CORE-011A and CORE-014 authorization-provider precedents), with crate placement and read/fetch contract shape as open design-phase decisions, and the concrete domain meaning either resolved with the user or explicitly scoped as a generic/abstract port with no concrete adapter (matching CORE-019's own non-goals pattern).
