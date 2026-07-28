# Delta for distributed-tracing

## ADDED Requirements

### Requirement: TraceContext Carries The Inbound Sampling Decision

`TraceContext` MUST carry the W3C sampling decision as a vendor-neutral value
(the `trace_flags` sampled bit), and SHOULD carry the inbound `tracestate` when
one is present. The sampling-decision type and the `tracestate` carrier MUST be
defined in `ego-domain` and MUST NOT reference any `opentelemetry` type — the
domain models only the `sampled` decision (`SamplingDecision { Sampled,
NotSampled }`) and treats `tracestate` as an opaque, size-bounded value it does
not parse. The `SamplingDecision → opentelemetry` flag/state mapping MUST live
ONLY in the `infrastructure` adapter.

#### Scenario: TraceContext exposes the honored sampling decision

- GIVEN a `TraceContext` constructed from a valid inbound `traceparent`
- WHEN the context's sampling decision is read
- THEN it exposes a `SamplingDecision` value equal to the inbound decision, and
  exposes the inbound `tracestate` when one was present

#### Scenario: The domain sampling-decision type carries no OpenTelemetry symbol

- GIVEN the `ego-domain` crate source, including `tracer.rs`
- WHEN the sampling-decision and tracestate types and `TraceContext`'s public
  signatures are inspected
- THEN no `opentelemetry`/`opentelemetry-otlp` type appears — the decision is a
  plain domain enum and `tracestate` is an opaque domain value

#### Scenario: The OTel flag mapping lives only in infrastructure

- GIVEN the `SamplingDecision → TraceFlags` and `tracestate → TraceState`
  mapping
- WHEN the workspace is inspected for where that mapping is defined
- THEN it exists ONLY in `crates/infrastructure`, and `crates/domain` contains
  no such mapping

### Requirement: Parent-Based Sampling Policy Honors The Inbound Decision

A parent-based sampling policy MUST honor a VALID inbound sampling decision: a
sampled inbound decision MUST remain `Sampled` and a not-sampled inbound
decision MUST remain `NotSampled`, end to end. The policy MUST NOT override a
valid inbound decision (the v1 always-on override is removed). When there is no
inbound parent, or the inbound `traceparent` is malformed and therefore
rejected, the decision MUST be a deterministic LOCAL ROOT decision that does
NOT depend on ambient state; the local-root default is `Sampled`.

#### Scenario: A valid sampled inbound decision stays sampled

- GIVEN an inbound `traceparent` whose flags byte indicates sampled (`-01`)
- WHEN a `TraceContext` is created from it and later serialized/exported
- THEN its sampling decision is `Sampled` and remains `Sampled` on the outbound
  header and the exported span — never re-derived or defaulted

#### Scenario: A valid not-sampled inbound decision stays not-sampled

- GIVEN an inbound `traceparent` whose flags byte indicates not-sampled (`-00`)
- WHEN a `TraceContext` is created from it and later serialized/exported
- THEN its sampling decision is `NotSampled` and is NOT overridden to sampled —
  the outbound header carries `-00` and the span is not force-exported as
  sampled

#### Scenario: A malformed inbound parent falls back to the local-root default

- GIVEN an inbound `traceparent` that is malformed (rejected by
  `parse_traceparent` with `TraceParseError::InvalidFormat`)
- WHEN the request proceeds and a local root context is started instead
- THEN the sampling decision is the deterministic local-root default
  (`Sampled`), and no decision is fabricated from the rejected input

#### Scenario: An absent inbound parent uses the local-root decision

- GIVEN a request with no inbound `traceparent` (a locally-originated root)
- WHEN `TraceContext::root()` is used
- THEN its sampling decision is the deterministic local-root default
  (`Sampled`), consulting no ambient state

### Requirement: Outbound Propagation And Export Reflect The Actual Decision

Outbound propagation and OTLP export MUST reflect the actual sampling decision
carried on the `TraceContext`, not a hardcoded value. `to_traceparent()` MUST
serialize `-01` when the decision is `Sampled` and `-00` when it is
`NotSampled`. The OTLP adapter MUST derive the exported span's `TraceFlags` and
`TraceState` from the domain decision/tracestate, and MUST NOT hardcode
`TraceFlags::SAMPLED` or `TraceState::NONE`.

#### Scenario: to_traceparent serializes the real decision

- GIVEN a `TraceContext` whose sampling decision is `NotSampled`
- WHEN `to_traceparent()` is called
- THEN the produced header ends in `-00`, not a hardcoded `-01`

#### Scenario: The OTLP adapter maps the decision instead of hardcoding sampled

- GIVEN a `TraceContext` whose decision is `NotSampled`
- WHEN the OTLP adapter builds the exported span
- THEN the span's `TraceFlags` reflect not-sampled (the parent-based policy does
  not export it as sampled), and the flags are NOT the hardcoded
  `TraceFlags::SAMPLED`

#### Scenario: A sampled decision is exported with faithful flags

- GIVEN a `TraceContext` whose decision is `Sampled` and which carries a
  non-empty inbound `tracestate`
- WHEN the OTLP adapter builds the exported span
- THEN the exported span's `TraceFlags` indicate sampled and its `TraceState`
  is derived from the carried `tracestate`, not `TraceState::NONE`

### Requirement: Inbound Sampling Interoperates With W3C And OpenTelemetry

The honored decision MUST round-trip through the W3C `traceparent` encoding: a
decision parsed from an inbound header and re-serialized via `to_traceparent()`
MUST encode the same `sampled` bit. The behavior MUST remain compatible with
OpenTelemetry — the exported `SpanData` remains valid and its `TraceFlags`
correspond to the domain decision via the infrastructure mapping.

#### Scenario: The decision round-trips across a service hop chain

- GIVEN service A emits a `traceparent` with a not-sampled decision (`-00`)
- WHEN B parses it via `from_inbound`, emits its own `to_traceparent()`, and C
  parses B's header
- THEN each hop preserves the not-sampled decision — B's and C's outbound
  headers both end in `-00`, and no hop re-samples the trace

#### Scenario: OTel export stays valid with a faithful sampled flag

- GIVEN a sampled `TraceContext`
- WHEN the OTLP adapter exports the span
- THEN the exported `SpanData` is a valid OpenTelemetry span whose `TraceFlags`
  indicate sampled, produced by the infrastructure `SamplingDecision →
  TraceFlags` mapping rather than a hardcoded constant

## MODIFIED Requirements

### Requirement: TraceContext Distinguishes Inbound Origination From Raw Parsing

`TraceContext::from_inbound(traceparent: &str) -> Result<TraceContext,
TraceParseError>` MUST produce a `TraceContext` with the SAME `trace_id` as the
parsed header, the remote span-id as `parent_span_id`, a NEW locally-generated
`span_id`, and the sampling decision (and `tracestate`, when present)
PRESERVED from the parsed header. This MUST be distinct from
`parse_traceparent`, which decodes `(TraceId, SpanId, SamplingDecision)` and
constructs no `TraceContext`. `parse_traceparent` MUST retain the sampled bit
decoded from the flags byte rather than validating and discarding it.

(Previously: `parse_traceparent` "only decodes `(TraceId, SpanId)` and
constructs no `TraceContext`", and the parsing scenario asserted it "returns
`(T, R)` only". The flags byte was validated then discarded, so `from_inbound`
carried no sampling decision. This change makes `parse_traceparent` return the
decoded `SamplingDecision` and `from_inbound` preserve it.)

#### Scenario: from_inbound creates a new local span with remote parent linkage and preserved decision

- GIVEN a valid `traceparent` header with `trace_id` `T`, remote `span_id` `R`,
  and a not-sampled flags byte
- WHEN `TraceContext::from_inbound(header)` is called
- THEN the result has `trace_id` `T`, `parent_span_id` `Some(R)`, a
  freshly-generated `span_id` different from `R`, and a sampling decision of
  `NotSampled` preserved from the header

#### Scenario: parse_traceparent decodes the identifiers and the sampling decision

- GIVEN the same valid `traceparent` header (with a sampled flags byte `-01`)
- WHEN `parse_traceparent(header)` is called
- THEN it returns `(T, R, SamplingDecision::Sampled)` — the identifiers plus the
  decoded decision, and still no new span-id and no `TraceContext`

#### Scenario: A→B→C parent linkage chains correctly

- GIVEN service A emits `to_traceparent()` for trace `T`, span `1`
- WHEN B calls `from_inbound` on that header (new span `2`, parent `1`) and
  emits its own `to_traceparent()`, and C calls `from_inbound` on B's header
- THEN C's `TraceContext` has `trace_id` `T`, `parent_span_id` `Some(2)`, and a
  span-id distinct from `1` and `2`

### Requirement: Out of Scope for v1

NOT covered by this capability in v1: OTLP-exported metrics, OTLP-exported
logs, actor/effect-runner and messaging trace origination/propagation,
client/server transport spans (outbound is propagation-only), CONFIGURABLE /
RATIO / PROBABILISTIC sampling (a runtime-tunable rate or `TraceIdRatioBased`
policy), a runtime-tunable local-root sample rate, and spans for CORE-012A
macro-guard denials (denials short-circuit before the interceptor chain;
accepted v1 limitation). Parent-based honoring of an inbound sampling decision
is IN scope (see the parent-based sampling requirement) and is NOT part of this
out-of-scope list. `Observability` and the CORE-012A denial-recording contract
remain unchanged.

(Previously: this requirement listed "configurable sampling" among the
out-of-scope items without distinction, consistent with the removed
"Sampling Is Always-On In v1" requirement. Parent-based honoring of the inbound
decision is now in scope; only configurable/ratio/probabilistic sampling and a
tunable local-root rate remain out of scope.)

#### Scenario: A macro-guard denial produces no span

- GIVEN an operation denied by `#[authorize]` or `#[tenant_scoped]` before
  reaching the interceptor chain
- WHEN the denial occurs
- THEN no span is started or ended, and the existing `Observability`
  denial-recording contract (CORE-012A) still records the denial exactly as
  before

#### Scenario: Configurable ratio sampling remains out of scope

- GIVEN this change
- WHEN evaluating conformance
- THEN no runtime-tunable sample rate or `TraceIdRatioBased`-style policy is
  required — only the parent-based honoring of an existing inbound decision,
  with a fixed local-root default, is in scope

## REMOVED Requirements

### Requirement: Sampling Is Always-On In v1

(Reason: This change supersedes the always-on override. The v1 requirement
mandated that "v1 MUST export every started/ended span (always-on sampling); no
sampler or sampling-decision hook MUST exist in the `Tracer` port or its
adapter" and that "parent-based sampling MUST NOT be implemented in v1" — all
three clauses are directly incompatible with honoring the inbound sampling
decision. The always-on behavior forced the sampled bit onto every outbound
header (`to_traceparent` hardcoding `-01`) and every exported span (the OTLP
adapter hardcoding `TraceFlags::SAMPLED`), silently overriding an upstream
service's not-sampled decision and breaking W3C Trace Context interoperability.)

(Migration: the parent-based sampling policy replaces always-on. A valid inbound
decision is now honored (sampled stays sampled, not-sampled stays not-sampled);
a locally-originated or malformed-parent request falls back to a deterministic
local-root default of `Sampled`, preserving always-on's effective behavior for
traces `ego-rs` originates. Consumers that relied on every span being exported
regardless of the inbound decision MUST instead rely on the local-root default
for locally-originated traces; there is no way to force-sample a trace an
upstream explicitly marked not-sampled. Configurable/ratio sampling remains out
of scope, so no new configuration surface is introduced.)
