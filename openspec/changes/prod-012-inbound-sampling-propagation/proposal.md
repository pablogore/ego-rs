# Proposal: PROD-012 — Inbound Sampling Propagation

## Intent

`ego-rs` v1 distributed tracing deliberately forces every span to be sampled
and exported: it froze an "always-on" sampling decision that (a) drops the
inbound W3C sampling flags entirely and (b) hardcodes the sampled bit on every
outbound header and every exported span. `parse_traceparent` validates the
`traceparent` flags byte but returns only `(TraceId, SpanId)`
(`crates/domain/src/tracer.rs:253`); `TraceContext` has no `trace_flags` /
`trace_state` field at all (`tracer.rs:143-147`); `to_traceparent` hardcodes
`-01` (`tracer.rs:188-189`); and the OTLP adapter hardcodes
`TraceFlags::SAMPLED` + `TraceState::NONE` (`crates/infrastructure/src/tracing_otlp.rs:288,290`).
There is no OpenTelemetry `Sampler`/`ParentBased`/`AlwaysOn` anywhere — the
adapter bypasses the SDK `TracerProvider`/`Sampler` and builds `SpanData`
directly. This breaks W3C Trace Context interoperability: an upstream service
that decided **not** to sample a trace has that decision silently overridden,
and downstream services see a fabricated `01` sampled flag. PROD-012 preserves
and honors the inbound sampling decision via a parent-based policy, keeping the
decision type OpenTelemetry-free in `ego-domain` and confining the OTel mapping
to the infrastructure adapter.

## Scope

### In Scope
- `TraceContext` carries the inbound sampling decision (`trace_flags`) and,
  where present, `tracestate`, as vendor-neutral domain values (no OTel types).
- `parse_traceparent` retains the sampled bit instead of discarding it.
- A parent-based sampling policy that honors a valid inbound sampled /
  not-sampled decision, with an explicit local-root fallback when no parent (or
  a malformed parent) is present.
- Outbound propagation (`to_traceparent`) reflects the actual decision instead
  of hardcoding `01`.
- The domain sampling-decision → OTel mapping (`TraceFlags`, `TraceState`,
  `ParentBased` sampler) confined to the `infrastructure` adapter.
- W3C Trace Context and OpenTelemetry compatibility for VALID / INVALID
  (malformed) / ABSENT (no parent) / NOT-SAMPLED inbound contexts.

### Out of Scope (Non-Goals / Follow-ups)
- Configurable / ratio / probabilistic (`TraceIdRatioBased`) sampling — only
  the parent-based honoring of an existing decision is added here.
- A runtime-tunable local-root sample rate (fixed default only; see design).
- gRPC and messaging inbound/outbound sampling propagation (no gRPC client
  transport; in-process messaging has no wire-header model — already out of
  scope in v1).
- HTTP client (outbound) spans — that is PROD-013, which depends on this change.
- Any change to span lifecycle, the in-flight span table, or the `Tracer` port
  signature.

## Frozen Decisions (decided constraints, not open questions)

1. **Honor, do not override.** A VALID inbound decision (sampled or
   not-sampled) MUST be preserved end-to-end. The always-on override is
   removed.
2. **Domain stays OpenTelemetry-free.** The sampling-decision type and the
   `tracestate` value live in `ego-domain` as vendor-neutral values; NO
   `opentelemetry` type may appear in `crates/domain`. The
   `SamplingDecision → TraceFlags`/`TraceState` and `ParentBased` sampler
   mapping lives ONLY in the `infrastructure` adapter (the sole `opentelemetry`
   consumer), consistent with the existing hexagonal boundary.
3. **Local root fallback is explicit.** When there is no inbound parent, or the
   inbound `traceparent` is malformed (rejected by `parse_traceparent`), the
   decision is a LOCAL ROOT decision, not an inherited one. The default MUST be
   deterministic and MUST NOT depend on ambient state.
4. **Supersede the v1 requirement openly.** This change contradicts the v1
   "Sampling Is Always-On In v1" requirement and its "parent-based sampling
   MUST NOT be implemented in v1" clause. The spec delta MUST REMOVE/MODIFY that
   requirement rather than silently contradict it.

## Open Fork for DESIGN (do not resolve here)

The local-root default when no valid inbound decision exists: **(A)** default
Sampled (preserve v1's current effective behavior for locally-originated
roots, so this change never starts dropping root traces), or **(B)** default
NotSampled. Design MUST decide consciously and justify the interop/observability
tradeoff.

## Capabilities

### New Capabilities
- None. This extends an existing capability.

### Modified Capabilities
- `distributed-tracing`: `TraceContext` gains a vendor-neutral sampling
  decision + optional `tracestate`; `parse_traceparent` retains the sampled
  bit; a parent-based sampling policy honors the inbound decision with a
  local-root fallback; outbound propagation and OTLP export reflect the actual
  decision. The v1 "Sampling Is Always-On" requirement is removed/superseded;
  the "Out of Scope" and inbound-parsing requirements are modified.

## Approach

Add a vendor-neutral `SamplingDecision` (and an opaque `tracestate` carrier) to
`ego-domain::tracer`. Extend `TraceContext` to carry the decision and
`tracestate`, and change `parse_traceparent` to return the decision alongside
`(TraceId, SpanId)`. `from_inbound` preserves the parsed decision; `root()`
applies the local-root fallback default. `to_traceparent` serializes the actual
decision (`-01` / `-00`). In `infrastructure`, replace the hardcoded
`TraceFlags::SAMPLED` + `TraceState::NONE` with a mapping from the domain
decision, and introduce the parent-based sampler mapping (the sampler lives in
infra, not domain). Outbound HTTP propagation already derives the header from
`to_traceparent()`, so it reflects the decision automatically once the domain
value is faithful.

## Affected Areas

| Area | Impact | Description |
|------|--------|-------------|
| `crates/domain/src/tracer.rs` | Modified | `SamplingDecision` enum + opaque `TraceState`; `TraceContext.trace_flags`/`trace_state` fields; `parse_traceparent` returns the decision; `from_inbound`/`root()`/`to_traceparent` honor it |
| `crates/infrastructure/src/tracing_otlp.rs` | Modified | Map `SamplingDecision → TraceFlags` and domain `tracestate → TraceState`; parent-based sampler mapping replacing hardcoded `SAMPLED`/`NONE` |
| `crates/transport` (outbound helper) | Verified | Header now reflects the real decision via `to_traceparent()`; no signature change |
| `openspec/specs/distributed-tracing/spec.md` (canonical) | Superseded (future) | v1 always-on requirement removed; inbound-parsing / out-of-scope requirements modified — via delta only, canonical not edited now |

## Risks

| Risk | Likelihood | Mitigation |
|------|------------|------------|
| OTel types leak into `ego-domain` via the decision type | Med | Domain holds a plain enum + opaque string; mapping confined to infra; grep gate in tasks |
| Removing always-on starts dropping traces unexpectedly | Med | Explicit local-root default (design fork A/B) documented; not-sampled only honored when the inbound decision is VALID |
| Malformed inbound header mis-parsed into a bogus decision | Low | `parse_traceparent` already rejects malformed headers → local-root fallback path, never a fabricated decision |
| Breaking the `parse_traceparent` public signature | Med | Documented compatibility note; callers updated in the same change; `from_inbound` remains the primary construction path |
| Contradicting the frozen v1 spec | High (by design) | Spec delta REMOVES/MODIFIES the v1 requirement explicitly (frozen decision 4) |

## Rollback Plan

Additive-then-substitutive at the value layer. To revert: drop the
`SamplingDecision`/`trace_state` fields and restore `parse_traceparent`'s
`(TraceId, SpanId)` return, `to_traceparent`'s `-01`, and the adapter's
`TraceFlags::SAMPLED` + `TraceState::NONE`. No schema/migration impact; the
span lifecycle and `Tracer` port are untouched, so rollback is behavior-neutral
except for restoring the always-on override.

## Dependencies

- Builds on PROD-003 distributed tracing (`TraceContext`, `parse_traceparent`,
  `to_traceparent`, the OTLP adapter). No dedicated open issue — this is a
  PROD-003 follow-up. (Issue #212 was the unrelated operation-naming follow-up
  and is already resolved; it does NOT cover this work.)
- Independent of other in-flight changes. PROD-013 (HTTP client spans) depends
  on THIS change (a client span must carry the honored decision).

## Success Criteria

- [ ] `TraceContext` exposes a vendor-neutral sampling decision (`trace_flags`)
      and optional `tracestate`, with zero `opentelemetry` types in
      `crates/domain`.
- [ ] `parse_traceparent` retains the inbound sampled bit rather than
      discarding it.
- [ ] A valid inbound sampled decision stays sampled; a valid inbound
      not-sampled decision stays not-sampled — end to end (parse → context →
      outbound header → OTLP export).
- [ ] Absent or malformed inbound parent falls back to the deterministic
      local-root default (design fork resolved).
- [ ] `to_traceparent` and the OTLP adapter reflect the actual decision — no
      hardcoded `01` / `TraceFlags::SAMPLED`.
- [ ] The v1 "Sampling Is Always-On" requirement is superseded via an explicit
      REMOVED/MODIFIED delta, not a silent contradiction.
- [ ] `cargo test --workspace` green; W3C + OTel compatibility scenarios pass.
