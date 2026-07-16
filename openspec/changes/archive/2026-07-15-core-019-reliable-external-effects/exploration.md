# Exploration: CORE-019 Reliable External Effects (+ CORE-019A Effect Readers)

## Current State

**Write-side external effects already have a value-type contract, but zero dispatcher.** `crates/domain/src/effect.rs` defines `Effect<E,R,S>::ExternalEffects(Vec<ExternalEffectDescription>)`, where `ExternalEffectDescription { idempotency_key, effect_type, payload, destination }` is documented as "described during handler execution... dispatched **after** the atomic commit succeeds. Handlers MUST NOT call external systems directly." `crates/runtime/src/interpreter.rs` defines `EffectInterpreter<E,R,S>` ("Implementations MUST match all Effect variants exhaustively"), but the *only* implementation in the whole workspace is a `#[cfg(test)]` `RecordingInterpreter` that just counts effects — no HTTP client, queue client, retry, or dispatch logic exists anywhere. `examples/reference-app` (CORE-018's "production reference service") never references `EffectInterpreter`/`ExternalEffectDescription` at all, confirmed by grep. This abstraction shipped under the pre-CORE-numbering `2026-06-22-effect-api` change; no canonical spec for it exists under `openspec/specs/` today.

**No reliability primitives exist anywhere** — grepped for retry/backoff/circuit-breaker/dispatch/outbox across `crates/`: zero hits. `IdempotencyKey` (`crates/domain/src/idempotency.rs`) only validates non-emptiness — no dedup-window/TTL/store concept.

**Existing "reader" precedent is synchronous and I/O-forbidding.** `crates/security-apikey/src/resolver.rs`'s `ApiKeyResolver::lookup` has a hard "MUST NOT perform I/O on any path" contract (timing side-channel prevention), enforced only by convention via a marker trait `LocalApiKeyResolver`. Its own doc explicitly anticipates the gap: "A hypothetical remote resolver SPI belongs behind a different, non-`LocalApiKeyResolver` type" — i.e., deliberately deferred, never built. The closer precedent is `KeyResolver` from `openspec/changes/archive/2026-06-23-CORE-011A-key-resolver-architecture/proposal.md`: made **async** even though the only shipped impl (`LocalKeyResolver`) is in-memory, "so CORE-011B can do network I/O without reshaping the trait." `crates/domain/src/auth/clock.rs`'s `Clock` trait is the one trivial existing "external-effect reader" (wall-clock time).

**Naming risk**: CORE-019's "External Effects" maps cleanly onto the existing `ExternalEffects` enum variant. But "Effect Readers" (CORE-019A) needs careful disambiguation from the existing write-side `Effect` vocabulary — it more plausibly names a read-only, I/O-capable port (the counterpart CORE-011A/`ApiKeyResolver` already anticipated but never built), not something that "reads" the `Effect` enum itself.

## Affected Areas
- `crates/domain/src/effect.rs` — existing `ExternalEffectDescription`/`Effect::ExternalEffects`, the write-side contract CORE-019 would harden
- `crates/domain/src/idempotency.rs` — `IdempotencyKey`; no dedup-window semantics yet
- `crates/runtime/src/interpreter.rs` — `EffectInterpreter` trait; only a test-only impl exists, no production dispatcher
- `examples/reference-app` — zero external-effect usage today; would be the dogfooding site per CORE-018/026 convention
- `crates/security-apikey/src/resolver.rs` — `ApiKeyResolver`/`LocalApiKeyResolver` precedent for reader-trait shape
- `crates/domain/src/auth/clock.rs` — `Clock`, the one existing minimal effect-read abstraction
- `openspec/specs/` — no canonical `effect`/`external-effects` spec exists yet to delta against

## Approaches
1. **Harden the existing `Effect`/`EffectInterpreter` seam with a real dispatcher + retry/idempotency** — Pros: reuses a frozen, tested contract; closes the gap effect-api's own spec already promised. Cons: `EffectInterpreter` exhaustively matches all variants — must scope carefully to just `ExternalEffects`. Effort: Medium.
2. **New async `EffectReader` trait family (CORE-019A)**, modeled on CORE-011A's `KeyResolver` (async trait, sync-bridging caller) rather than `ApiKeyResolver`'s no-I/O contract — Pros: fills a gap the codebase already flagged as deferred; parallels an accepted pattern. Cons: naming collision risk with `Effect` enum; needs an explicit ADR on scope vs. per-crate resolvers. Effort: Medium-High.
3. **Split CORE-019/CORE-019A using the CORE-011/011A and CORE-018/018a/018b lettered-subscope convention** (Metadata table: Change ID, Type "Amendment to CORE-0XX", Parent, Enables, Status) — matches every existing precedent in this repo; requires resolving during sdd-propose whether 019A is dependent-sequenced (like 011→011A) or independent/parallel (like 018a/018b).

## Recommendation
Scope CORE-019 (main) as hardening the existing write-side `Effect::ExternalEffects`/`EffectInterpreter` seam with real dispatch + retry/idempotency-window semantics, and CORE-019A as a new async, I/O-capable reader trait family modeled on CORE-011A's `KeyResolver` precedent. Use the CORE-011A/CORE-018a Metadata-table convention to make the parent/subscope relationship explicit, and resolve the naming-collision risk against the existing `Effect` enum during sdd-propose.

## Risks
- Naming collision between "Effect Reader" and the already-shipped `Effect` enum (write-side outcomes) needs explicit disambiguation in the proposal.
- Zero existing dispatcher — this is greenfield reliability engineering (retry/backoff/circuit-breaking), not a refactor of partial code.
- No canonical spec exists for the `effect` capability yet; sdd-spec will need to backfill a spec file that has never existed under `openspec/specs/`, a larger-than-usual spec-phase scope.
- `IdempotencyKey` has no dedup-window/TTL/store semantics defined anywhere — CORE-019's reliability claims need that decision made explicitly.
- Open question for the user before sdd-propose: is CORE-019A dependent-sequenced after CORE-019 (like CORE-011→011A) or independent (like CORE-018a/018b)?
