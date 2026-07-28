# Design: PROD-012 — Inbound Sampling Propagation

## Technical Approach

The inbound W3C sampling decision is dropped at three points today and forced
to always-sampled at two more:

- `parse_traceparent` validates the flags byte but returns only
  `(TraceId, SpanId)` — the sampled bit is parsed then discarded
  (`crates/domain/src/tracer.rs:253`, with the flags validated at
  `tracer.rs:218-225`).
- `TraceContext` has no field to carry a sampling decision or `tracestate`
  (`crates/domain/src/tracer.rs:143-147` — only `trace_id`, `span_id`,
  `parent_span_id`).
- `to_traceparent` hardcodes the sampled flag `-01`
  (`crates/domain/src/tracer.rs:188-189`), with a doc line that literally reads
  "Always sampled (`01`) — v1 sampling is always-on (ADR-8)".
- The OTLP adapter hardcodes `TraceFlags::SAMPLED` and `TraceState::NONE` when
  building the exported `SpanData`
  (`crates/infrastructure/src/tracing_otlp.rs:288,290`).
- There is no OpenTelemetry `Sampler`/`ParentBased`/`AlwaysOn` anywhere: the
  adapter builds `SpanData` directly and bypasses the SDK
  `TracerProvider`/`Sampler` (`crates/infrastructure/src/tracing_otlp.rs:280-291`
  — "no SDK `Tracer`/`IdGenerator` involved").

PROD-012 introduces a vendor-neutral `SamplingDecision` and an opaque
`tracestate` carrier in `ego-domain::tracer`, threads both through
`TraceContext`, `parse_traceparent`, `from_inbound`, `root()`, and
`to_traceparent`, and moves the OTel mapping (flags, tracestate, and a
`ParentBased` sampler) into the `infrastructure` adapter — the sole
`opentelemetry` consumer (crates pinned at `opentelemetry* = 0.32`,
`crates/infrastructure/Cargo.toml:16-18`). The domain never sees an OTel type;
the adapter never invents a decision.

## Architecture Decisions

### ADR-1 (DECISION 1): Where the sampling-decision type lives → **Domain enum + opaque tracestate; OTel mapping in infra**

**Choice**: define `SamplingDecision { Sampled, NotSampled }` and an opaque
`TraceState(String)` newtype in `ego-domain::tracer`. `TraceContext` carries a
`trace_flags: SamplingDecision` (the W3C `sampled` bit, the only flag v1
models) and `trace_state: Option<TraceState>`. The
`SamplingDecision → opentelemetry::trace::TraceFlags` and
`domain::TraceState → opentelemetry::trace::TraceState` mapping lives ONLY in
`crates/infrastructure/src/tracing_otlp.rs`.

**Rejected**: (a) storing `opentelemetry::trace::TraceFlags`/`TraceState`
directly on `TraceContext` — leaks OTel into `ego-domain`, violating the
hexagonal boundary the tracing capability already enforces
("Tracer Port Is Transport-Agnostic": no `opentelemetry` type in `tracer.rs`);
(b) a raw `u8` flags byte on the context — re-admits the W3C wire encoding into
the domain and invites callers to reinterpret bits the domain does not model.

**Rationale**:

| Option | Tradeoff | Verdict |
|---|---|---|
| Domain enum + opaque tracestate | Matches the existing rule that `crates/domain` holds vendor-neutral values only; `TraceId`/`SpanId` are already domain newtypes with hex (de)serialization, so a `SamplingDecision` enum + `TraceState` newtype is the same shape. Infra maps once. | **Chosen** |
| OTel `TraceFlags`/`TraceState` on the context | Zero mapping code, but breaks "Domain crate has no opentelemetry symbols" — the very scenario the tracing spec asserts. | Rejected |
| Raw `u8` flags byte | Smallest struct, but pushes W3C bit semantics into every domain caller and loses the "only `sampled` is modeled" clarity. | Rejected |

`tracestate` is modeled as an opaque, size-bounded string the domain does not
parse (W3C treats it as an ordered vendor list). The domain only carries and
re-serializes it; the adapter maps it to `opentelemetry::trace::TraceState`.
This keeps `crates/domain` free of OTel and of tracestate grammar.

### ADR-2 (DECISION 2): Parent-based sampler behavior + local-root fallback → **Honor valid inbound; default Sampled at a local root (fork A)**

**Choice**: a parent-based policy. When `from_inbound` parses a VALID
`traceparent`, the parsed `SamplingDecision` is preserved verbatim — a sampled
parent stays `Sampled`, a not-sampled parent stays `NotSampled`. When there is
no inbound parent (`root()`) OR the inbound header is malformed (rejected by
`parse_traceparent`, so `from_inbound` returns `Err` and the caller starts a
local root), the decision is a LOCAL ROOT decision defaulting to `Sampled`
(fork **A**). The parent-based sampler in `infrastructure` maps
`SamplingDecision::Sampled → RecordAndSample` and
`SamplingDecision::NotSampled → Drop`, i.e. an OTel `ParentBased` with an
`AlwaysOn` root delegate.

**Rejected**: (B) default `NotSampled` at a local root — would start dropping
every locally-originated root trace the moment always-on is removed, a silent
observability regression with no operator opt-in; deferred with configurable
ratio sampling (out of scope). Also rejected: "always defer / never decide" —
v1 has no downstream sampler to defer to, so a decision must be made at the
boundary.

**Rationale**: fork A preserves v1's *effective* behavior for traces `ego-rs`
originates (roots stay sampled), so the only behavior change is the intended
one — an explicit upstream not-sampled decision is now honored instead of
overridden. The malformed case collapses into the local-root path precisely
because `parse_traceparent` already rejects malformed headers
(`crates/domain/src/tracer.rs:218-252`), so a bogus decision can never be
fabricated from bad input.

**Four inbound cases (normative):**

| Inbound | Parse result | Resulting decision | Rationale |
|---|---|---|---|
| VALID, sampled (`-01`) | `Ok((T, R, Sampled))` | `Sampled` | Honor upstream |
| VALID, not-sampled (`-00`) | `Ok((T, R, NotSampled))` | `NotSampled` | Honor upstream |
| INVALID (malformed) | `Err(InvalidFormat)` | local root → `Sampled` (default) | Never trust bad input; fall back |
| ABSENT (no parent) | n/a — `root()` | local root → `Sampled` (default) | Local origination decision |

### ADR-3 (DECISION 3): How the delta supersedes the v1 always-on requirement → **REMOVE always-on; MODIFY the parsing and out-of-scope requirements**

**Choice**: the `distributed-tracing` spec delta:
- **REMOVES** "Sampling Is Always-On In v1" (canonical
  `openspec/specs/distributed-tracing/spec.md:271-281`), whose text mandates
  always-on and states "no sampler or sampling-decision hook MUST exist" and
  "parent-based sampling MUST NOT be implemented in v1" — directly incompatible
  with this change. The REMOVED entry carries an explicit `(Reason: ...)` and
  `(Migration: ...)`.
- **MODIFIES** "TraceContext Distinguishes Inbound Origination From Raw
  Parsing" (canonical `:113-138`), whose scenario asserts `parse_traceparent`
  "returns `(T, R)` only — no new span-id, no `TraceContext`". It now returns
  the sampling decision too, and `from_inbound` preserves it. Restated in full
  with `(Previously: ...)`.
- **MODIFIES** "Out of Scope for v1" (canonical `:283-298`), which lists
  "configurable sampling" as out of scope. Parent-based honoring of the inbound
  decision moves IN scope; configurable/ratio sampling stays out. Restated in
  full with `(Previously: ...)`.

**Rejected**: leaving the always-on requirement in place and adding a
contradicting requirement — OpenSpec deltas MUST NOT silently contradict a
requirement they supersede (frozen decision 4). A REMOVE + targeted MODIFY
keeps the canonical spec internally consistent once archived.

**Rationale**: the three canonical requirements above are the only ones whose
literal text conflicts with honoring the inbound decision; the span-lifecycle,
port-neutrality, and export requirements are unaffected and stay as ADDED-only
neighbors are not needed. The outbound-propagation requirement's *text* does
not pin the flag value (it only says the header equals `to_traceparent()`), so
it is covered by an ADDED requirement fixing what `to_traceparent()` must now
emit rather than a MODIFY.

## Data Flow

    Inbound HTTP request
      │  traceparent: 00-<tid>-<sid>-<flags>
      ▼
    parse_traceparent(header)  [crates/domain/src/tracer.rs]
      ├─ malformed ──▶ Err(InvalidFormat) ──▶ TraceContext::root() ──▶ local-root default (Sampled)
      └─ Ok((T, R, decision))
             ▼
    TraceContext::from_inbound(header)
      { trace_id=T, span_id=NEW, parent_span_id=Some(R),
        trace_flags=decision, trace_state=<parsed tracestate | None> }
             │  (carried by value on ServiceContext.trace_context())
             ├───────────────▶ to_traceparent()  ─▶  00-<tid>-<sid>-<01|00>   (reflects decision, not hardcoded 01)
             └───────────────▶ OTLP adapter [crates/infrastructure/src/tracing_otlp.rs]
                                  SamplingDecision → TraceFlags (Sampled⇒SAMPLED, NotSampled⇒0x00)
                                  domain TraceState → opentelemetry::TraceState
                                  ParentBased{AlwaysOn root}: NotSampled ⇒ span dropped, Sampled ⇒ exported

### Sequence: honoring a not-sampled inbound decision A→B

    Service A                 B: parse            B: TraceContext        B: outbound / export
      │  traceparent 00-T-1-00 ▶│                                       
      │                         │ parse_traceparent ⇒ Ok((T,1,NotSampled))
      │                         │────────────────────▶│ from_inbound   
      │                         │                      │ trace_flags=NotSampled, parent=1, span=2
      │                         │                      │────────────────────▶│ to_traceparent ⇒ 00-T-2-00
      │                         │                      │                      │ OTLP: ParentBased ⇒ Drop (not exported)
      ◀── downstream sees 00-T-2-00 (still not-sampled) ─────────────────────┤

## File Changes

_All rows are FUTURE production work planned by this change; nothing is
implemented in this planning phase._

| File | Action | Description |
|------|--------|-------------|
| `crates/domain/src/tracer.rs` | Modify | Add `SamplingDecision { Sampled, NotSampled }` and opaque `TraceState(String)`; add `trace_flags`/`trace_state` fields to `TraceContext`; change `parse_traceparent` to return the decision; preserve it in `from_inbound`; apply the local-root default in `root()`/`child()`; make `to_traceparent` serialize the actual flag; update doc lines (remove "Always sampled (`01`)") |
| `crates/infrastructure/src/tracing_otlp.rs` | Modify | Replace hardcoded `TraceFlags::SAMPLED` (`:288`) and `TraceState::NONE` (`:290`) with a map from the domain decision/tracestate; add the `SamplingDecision`-driven parent-based sampler mapping (`Sampled ⇒ export`, `NotSampled ⇒ drop`) |
| `crates/infrastructure/src/tracing_otlp.rs` (or new `sampling.rs`) | Create/Modify | `fn to_otel_trace_flags(SamplingDecision) -> TraceFlags` and `fn to_otel_trace_state(&Option<domain::TraceState>) -> TraceState` mapping helpers — the sole OTel-aware sampling code |
| `crates/transport` (outbound propagation helper) | Verify/Modify | Confirm the outbound header is `ctx.trace_context().to_traceparent()` and now reflects the real decision; no signature change |
| `openspec/specs/distributed-tracing/spec.md` | Superseded (future, via archive) | Canonical spec updated by this delta on archive — NOT edited during planning |

## Interfaces / Contracts

```rust
// ego-domain::tracer — vendor-neutral, ZERO opentelemetry types

/// The W3C `sampled` decision carried on a TraceContext. v1 models only the
/// `sampled` flag bit; other W3C flag bits are not represented.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SamplingDecision {
    /// The trace is sampled (`sampled` flag set; wire `-01`).
    Sampled,
    /// The trace is explicitly not sampled (`sampled` flag clear; wire `-00`).
    NotSampled,
}

/// Opaque W3C `tracestate` carrier. The domain does NOT parse the vendor list;
/// it is carried and re-serialized verbatim. Size-bounded per W3C.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TraceState(String);

pub struct TraceContext {
    trace_id: TraceId,
    span_id: SpanId,
    parent_span_id: Option<SpanId>,
    trace_flags: SamplingDecision,   // NEW — the honored decision
    trace_state: Option<TraceState>, // NEW — SHOULD carry when present
}

impl TraceContext {
    /// Local root: no parent, decision defaults to the local-root default
    /// (ADR-2 fork A ⇒ Sampled). No ambient state consulted.
    pub fn root() -> Self;

    /// Preserves the parsed decision + tracestate from the inbound header.
    pub fn from_inbound(traceparent: &str) -> Result<Self, TraceParseError>;

    /// Serializes the ACTUAL decision: `-01` when Sampled, `-00` when NotSampled.
    /// (No longer hardcodes `-01`.)
    pub fn to_traceparent(&self) -> String;

    pub fn sampling_decision(&self) -> SamplingDecision; // NEW accessor
    pub fn trace_state(&self) -> Option<&TraceState>;    // NEW accessor
}

/// BREAKING: now returns the sampling decision decoded from the flags byte,
/// instead of validating-then-discarding it. Callers updated in this change.
pub fn parse_traceparent(s: &str)
    -> Result<(TraceId, SpanId, SamplingDecision), TraceParseError>;
```

```rust
// crates/infrastructure/src/tracing_otlp.rs — the ONLY OTel-aware mapping
fn to_otel_trace_flags(d: SamplingDecision) -> opentelemetry::trace::TraceFlags {
    match d { SamplingDecision::Sampled => TraceFlags::SAMPLED,
              SamplingDecision::NotSampled => TraceFlags::default() /* 0x00 */ }
}
// domain TraceState (opaque) -> opentelemetry::trace::TraceState (parsed by OTel)
// ParentBased{ root: AlwaysOn }: NotSampled ⇒ span not exported; Sampled ⇒ exported.
```

## Error Model

No new error variant. Malformed inbound headers continue to yield
`TraceParseError::InvalidFormat` from `parse_traceparent`
(`crates/domain/src/tracer.rs:256-261`); the caller responds by starting a
local root (`TraceContext::root()`), so a parse failure degrades to the
local-root default rather than a fabricated decision. `parse_traceparent`
never returns a `SamplingDecision` for input it rejects.

## Observability

No new metrics or labels. The honored decision is visible on the exported
span's `TraceFlags` and in the outbound `traceparent` header. No raw ids,
payloads, tenant ids, or credentials are added anywhere; `tracestate` is
carried opaquely and is not logged.

## Testing Strategy

| Layer | What | Approach |
|-------|------|----------|
| Unit (domain) | `parse_traceparent` returns `Sampled` for `-01` and `NotSampled` for `-00`; malformed still `Err` | `crates/domain/src/tracer.rs` tests |
| Unit (domain) | `from_inbound` preserves the parsed decision + tracestate; `root()` yields the local-root default (Sampled) | domain tests |
| Unit (domain) | `to_traceparent` emits `-01` for Sampled and `-00` for NotSampled (round-trips through parse) | domain tests |
| Unit (domain) | No `opentelemetry` symbol appears in `crates/domain` (grep gate) | build/grep |
| Unit (infra) | `to_otel_trace_flags(NotSampled) == 0x00`, `(Sampled) == SAMPLED`; domain tracestate maps to `opentelemetry::TraceState` | `crates/infrastructure` tests |
| Integration (infra) | A NotSampled decision is not exported (ParentBased drop); a Sampled decision is exported | `#[tokio::test]` against the export test harness |
| Compat | A→B→C: a not-sampled decision from A survives to C's outbound header; a sampled decision likewise | round-trip test |

## Threat Matrix

N/A — no routing, shell, subprocess, VCS/PR automation, or process-integration
boundary. The only data crossing a trust boundary is the inbound `traceparent`,
which `parse_traceparent` already strictly validates (version `00`, lowercase
hex, non-zero ids) before any decision is derived; malformed input degrades to
the local-root default and can never fabricate a decision. `tracestate` is
carried opaquely, size-bounded, and never logged or interpreted in the domain.

## Migration / Rollout / Compatibility

`parse_traceparent`'s return type changes from `(TraceId, SpanId)` to
`(TraceId, SpanId, SamplingDecision)` — a public breaking change. Blast radius
is small: `from_inbound` is the primary caller (`crates/domain/src/tracer.rs:166`)
and is updated in the same change; any other caller destructures one extra
tuple element. `TraceContext` gains two fields but its constructors
(`root`/`from_inbound`/`child`) keep their signatures, so external construction
paths are unchanged. On the wire, W3C compatibility improves: downstream
services stop receiving a fabricated `01` when the upstream chose `00`. OTel
compatibility is preserved — the adapter still emits valid `SpanData`, now with
faithful `TraceFlags`/`TraceState`. Rollback restores the always-on override
(see proposal Rollback Plan). No schema/migration.

## Open Questions

None blocking. The local-root default is resolved as fork A (Sampled) in ADR-2;
a configurable local-root ratio is an explicit follow-up (out of scope). The
exact home of the infra mapping helpers (inline in `tracing_otlp.rs` vs a new
`sampling.rs`) is an apply-time detail that does not affect the contract.
