//! Tracer port - the domain contract for distributed-tracing span lifecycle.
//!
//! `Tracer` is transport-agnostic and vendor-neutral: no `opentelemetry` (or
//! any other vendor) type appears anywhere in this module's public
//! signatures. Trace identity travels explicitly as data on `TraceContext`
//! (carried by value on `ServiceContext`) — never via ambient/task-local
//! state (`Context::current()`-style lookups are forbidden by the enforced
//! architecture invariant).
//!
//! ## Responsibility
//!
//! - Carry W3C-compatible trace/span identity (`TraceId`, `SpanId`,
//!   `TraceContext`)
//! - Parse/serialize the W3C `traceparent` header
//! - Provide a redaction-safe, typed `SpanAttributes` allow-list so
//!   sensitive data (tenant ids, credentials, principal subject, arbitrary
//!   payloads) is structurally unrepresentable
//! - Define the `Tracer` span start/end port and a separate
//!   `TracerLifecycle` shutdown port (ADR-9)
//!
//! ## Non-responsibility
//!
//! - Export/transport of spans (OTLP wiring lives in `infrastructure`)
//! - Sampling decisions (v1 is always-on; see ADR-8)
//! - Nested/manual span orchestration (`child()` is a seam only; v1 has
//!   exactly one interceptor-owned span per request boundary)
//!
//! ## Non-blocking
//!
//! `Tracer` implementations MUST NOT perform synchronous I/O or network calls
//! inside any trait method, and MUST NOT hold a contended/global lock across
//! exporter/SDK work; span bookkeeping MUST stay bounded and short-lived. This
//! is the precise property an exporter-backed adapter must satisfy — a
//! per-shard concurrent map for bookkeeping is fine; a global mutex held
//! across span construction/export is not. (A sharper statement of
//! `Observability`'s non-blocking intent.)

use rand::RngCore;
use std::time::Duration;

// ---------------------------------------------------------------------------
// TraceId / SpanId
// ---------------------------------------------------------------------------

/// A W3C-compatible 16-byte trace identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TraceId([u8; 16]);

impl TraceId {
    /// Generate a new random `TraceId`.
    pub fn new() -> Self {
        let mut bytes = [0u8; 16];
        rand::thread_rng().fill_bytes(&mut bytes);
        Self(bytes)
    }

    /// Render as 32 lowercase hex characters (W3C `trace-id` field).
    pub fn to_hex(self) -> String {
        encode_hex(&self.0)
    }

    /// Parse from 32 lowercase/uppercase hex characters (tolerant, internal).
    pub fn from_hex(s: &str) -> Result<Self, TraceParseError> {
        let mut bytes = [0u8; 16];
        decode_hex(s, &mut bytes)?;
        Ok(Self(bytes))
    }

    /// True if all 16 bytes are zero — W3C forbids an all-zero `trace-id`.
    pub fn is_zero(&self) -> bool {
        self.0 == [0u8; 16]
    }
}

impl Default for TraceId {
    fn default() -> Self {
        Self::new()
    }
}

/// A W3C-compatible 8-byte span identifier — the span handle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SpanId([u8; 8]);

impl SpanId {
    /// Generate a new random `SpanId`.
    pub fn new() -> Self {
        let mut bytes = [0u8; 8];
        rand::thread_rng().fill_bytes(&mut bytes);
        Self(bytes)
    }

    /// Render as 16 lowercase hex characters (W3C `parent-id` field).
    pub fn to_hex(self) -> String {
        encode_hex(&self.0)
    }

    /// Parse from 16 lowercase/uppercase hex characters (tolerant, internal).
    pub fn from_hex(s: &str) -> Result<Self, TraceParseError> {
        let mut bytes = [0u8; 8];
        decode_hex(s, &mut bytes)?;
        Ok(Self(bytes))
    }

    /// True if all 8 bytes are zero — W3C forbids an all-zero `parent-id`.
    pub fn is_zero(&self) -> bool {
        self.0 == [0u8; 8]
    }
}

impl Default for SpanId {
    fn default() -> Self {
        Self::new()
    }
}

fn encode_hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{:02x}", b));
    }
    s
}

fn decode_hex(s: &str, out: &mut [u8]) -> Result<(), TraceParseError> {
    if s.len() != out.len() * 2 {
        return Err(TraceParseError::InvalidFormat);
    }
    for (i, byte) in out.iter_mut().enumerate() {
        let hex_pair = &s[i * 2..i * 2 + 2];
        *byte = u8::from_str_radix(hex_pair, 16).map_err(|_| TraceParseError::InvalidFormat)?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// TraceContext
// ---------------------------------------------------------------------------

/// Explicit trace-context value: carried by value on `ServiceContext`, never
/// via ambient/task-local state (ADR-1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TraceContext {
    trace_id: TraceId,
    span_id: SpanId,
    parent_span_id: Option<SpanId>,
}

impl TraceContext {
    /// Start a brand-new trace with a root span (`parent_span_id` is `None`).
    pub fn root() -> Self {
        Self {
            trace_id: TraceId::new(),
            span_id: SpanId::new(),
            parent_span_id: None,
        }
    }

    /// Build a `TraceContext` from an inbound W3C `traceparent` header.
    ///
    /// Keeps the SAME `trace_id` as the parsed header, sets the remote
    /// span-id as `parent_span_id`, and generates a NEW local `span_id`.
    /// Distinct from [`parse_traceparent`], which only decodes the raw
    /// `(TraceId, SpanId)` pair and constructs no `TraceContext`.
    pub fn from_inbound(traceparent: &str) -> Result<Self, TraceParseError> {
        let (trace_id, remote_span_id) = parse_traceparent(traceparent)?;
        Ok(Self {
            trace_id,
            span_id: SpanId::new(),
            parent_span_id: Some(remote_span_id),
        })
    }

    /// Derive a child context: same trace, parent = `self.span_id`, new
    /// local span. Future-nesting seam — v1 does not call this (exactly one
    /// interceptor-owned span per request boundary).
    pub fn child(&self) -> Self {
        Self {
            trace_id: self.trace_id,
            span_id: SpanId::new(),
            parent_span_id: Some(self.span_id),
        }
    }

    /// Serialize the CURRENT LOCAL span (`trace_id`/`span_id`) as a W3C
    /// `traceparent` header, so it becomes the remote parent of the next
    /// service. Always sampled (`01`) — v1 sampling is always-on (ADR-8).
    pub fn to_traceparent(&self) -> String {
        format!("00-{}-{}-01", self.trace_id.to_hex(), self.span_id.to_hex())
    }

    /// The span handle already carried by this context.
    pub fn span_id(&self) -> SpanId {
        self.span_id
    }

    /// The trace this context belongs to.
    pub fn trace_id(&self) -> TraceId {
        self.trace_id
    }

    /// The parent span-id, if any (remote parent for `from_inbound`, local
    /// parent for `child()`, `None` for `root()`).
    pub fn parent_span_id(&self) -> Option<SpanId> {
        self.parent_span_id
    }
}

/// Raw W3C `traceparent` decode only — no `TraceContext` is constructed.
///
/// Format: `{version:2 hex}-{trace-id:32 hex}-{parent-id:16 hex}-{flags:2 hex}`.
pub fn parse_traceparent(s: &str) -> Result<(TraceId, SpanId), TraceParseError> {
    let parts: Vec<&str> = s.split('-').collect();
    let [version, trace_id_hex, span_id_hex, flags]: [&str; 4] = parts
        .try_into()
        .map_err(|_| TraceParseError::InvalidFormat)?;

    if version.len() != 2 || flags.len() != 2 {
        return Err(TraceParseError::InvalidFormat);
    }
    if !version.bytes().all(|b| b.is_ascii_hexdigit())
        || !flags.bytes().all(|b| b.is_ascii_hexdigit())
    {
        return Err(TraceParseError::InvalidFormat);
    }
    // EGO v1 supports exactly traceparent version `00` (the only version whose
    // format it serializes). Anything else — the W3C-forbidden `ff`, other
    // versions (`01`..`fe`), or uppercase (`0A`/`FF`) — is rejected rather than
    // guessing a forward-compatible interpretation we do not yet need.
    if version != "00" {
        return Err(TraceParseError::InvalidFormat);
    }
    // Version 00 mandates lowercase hex for the id/flags fields. Keep
    // `from_hex` tolerant for internal use; enforce strictly at this inbound
    // boundary so a non-conformant remote header cannot become an EGO identity.
    if [trace_id_hex, span_id_hex, flags]
        .iter()
        .any(|f| f.bytes().any(|b| b.is_ascii_uppercase()))
    {
        return Err(TraceParseError::InvalidFormat);
    }

    let trace_id = TraceId::from_hex(trace_id_hex)?;
    // W3C forbids an all-zero trace-id.
    if trace_id.is_zero() {
        return Err(TraceParseError::InvalidFormat);
    }
    let span_id = SpanId::from_hex(span_id_hex)?;
    // W3C forbids an all-zero parent-id (span-id).
    if span_id.is_zero() {
        return Err(TraceParseError::InvalidFormat);
    }
    Ok((trace_id, span_id))
}

/// Error returned when a W3C `traceparent` header is malformed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TraceParseError {
    /// The header did not match the W3C `traceparent` format.
    InvalidFormat,
}

impl std::fmt::Display for TraceParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidFormat => write!(f, "invalid W3C traceparent header"),
        }
    }
}

impl std::error::Error for TraceParseError {}

// ---------------------------------------------------------------------------
// SpanAttributes / SpanOutcome
// ---------------------------------------------------------------------------

/// Redaction-safe allow-list of span attributes (ADR-6).
///
/// This is the ONLY way to attach attributes to a span. There is
/// deliberately no constructor or field for a raw tenant id, a
/// credential/token, a principal subject, or an arbitrary payload — such
/// data cannot be expressed as `SpanAttributes`, so redaction is enforced
/// structurally at this type rather than by a runtime filter in the
/// adapter.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SpanAttributes {
    tenant_present: Option<bool>,
    duration: Option<Duration>,
    /// The **redacted** operation key (AD-10), typed so the raw one cannot be
    /// put here. See [`SpanAttributes::with_operation_key_hash`].
    operation_key_hash: Option<crate::operation::OperationKeyHash>,
}

impl SpanAttributes {
    /// Start a new, empty attribute set. A span's operation is its **name**
    /// (the `name` argument to [`Tracer::start_span`]) — not an attribute —
    /// so no operation name is stored here.
    pub fn new() -> Self {
        Self::default()
    }

    /// Record only whether an inbound tenant *hint* was present — never the
    /// tenant id itself, and never the resolved/canonical tenant.
    ///
    /// This is set at `on_request` (pre-enforcement) from
    /// `ServiceContext::has_tenant_hint()`, so it reflects the caller-supplied
    /// ingress hint, NOT the authoritative tenant produced by resolution.
    pub fn with_tenant_hint_present(mut self, present: bool) -> Self {
        self.tenant_present = Some(present);
        self
    }

    /// Record the span's duration.
    pub fn with_duration(mut self, d: Duration) -> Self {
        self.duration = Some(d);
        self
    }

    /// Whether a tenant was present, if recorded.
    pub fn tenant_present(&self) -> Option<bool> {
        self.tenant_present
    }

    /// The recorded duration, if any.
    pub fn duration(&self) -> Option<Duration> {
        self.duration
    }

    /// Record the **redacted** operation key for an idempotency span (AD-10).
    ///
    /// Takes an [`OperationKeyHash`](crate::operation::OperationKeyHash), not a
    /// string. That type's only constructor hashes an `OperationKey`, so a raw
    /// client-supplied key is not something this method can be handed — the
    /// redaction is a property of the signature rather than of every call site
    /// remembering to hash first. There is deliberately no `with_operation_key`
    /// and no `impl Into<String>` overload; either would reopen exactly the path
    /// this type exists to close.
    ///
    /// Emitted as the span attribute `idempotency.operation_key_hash`. It is a
    /// span attribute **only** — never a metric one, because the value is
    /// unbounded and would multiply time series without limit. That is not left
    /// to discipline either:
    /// [`Observability::metric`](crate::observability::Observability::metric)
    /// takes a name and a value and has no attribute parameter, so there is no
    /// metric dimension for it to become.
    pub fn with_operation_key_hash(mut self, hash: crate::operation::OperationKeyHash) -> Self {
        self.operation_key_hash = Some(hash);
        self
    }

    /// The redacted operation key, if recorded.
    pub fn operation_key_hash(&self) -> Option<&str> {
        self.operation_key_hash.as_ref().map(|h| h.as_str())
    }
}

/// The outcome a span ends with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpanOutcome {
    /// The operation completed successfully.
    Ok,
    /// The operation failed. `status_message` MUST be redaction-safe (no
    /// raw sensitive values).
    Error {
        /// A redaction-safe status message describing the failure.
        status_message: String,
    },
}

// ---------------------------------------------------------------------------
// Tracer / TracerLifecycle
// ---------------------------------------------------------------------------

/// Tracer port - the domain contract for span start/end lifecycle.
///
/// Transport-agnostic: no `opentelemetry` (or other vendor) type appears in
/// this trait's signature. Implementations (adapters) live in the
/// infrastructure layer.
///
/// # Non-blocking
///
/// Implementations MUST NOT perform synchronous I/O or network calls inside
/// any method on this trait, and MUST NOT hold a contended/global lock across
/// exporter/SDK work; span bookkeeping MUST stay bounded and short-lived.
pub trait Tracer: Send + Sync {
    /// Start a span for `ctx`. Returns nothing: the authoritative span
    /// identity is `ctx.span_id()` (`TraceContext::span_id()`), so there is
    /// no separate returned handle/token. A previously-returned `SpanId`
    /// would always equal `ctx.span_id()` and the stateless interceptor
    /// discards it; `end_span` re-derives the id from `&ctx` instead.
    fn start_span(&self, ctx: &TraceContext, name: &str, attrs: SpanAttributes);

    /// End the span identified by `span`.
    ///
    /// `end_span` MUST be idempotent per `SpanId`: after the first terminal
    /// outcome, subsequent calls for the same `SpanId` MUST have no effect.
    /// This is why the interceptor may safely call `end_span` on both
    /// `on_response` and `on_error` for the same request boundary.
    ///
    /// The enforcing adapter contract test (an `OtlpTracer` that actually
    /// closes-once and drops duplicate ends) lands in PR5 — this port only
    /// states the normative contract; no adapter is implemented here.
    fn end_span(&self, span: SpanId, outcome: SpanOutcome);
}

/// Exporter/operational lifecycle for a `Tracer` — SEPARATE from the
/// domain tracing port (ADR-9), so `NoopTracer`, test spies, and future
/// tracers are not forced to know an OTLP operational concern.
pub trait TracerLifecycle: Send + Sync {
    /// Flush all pending (unended) spans and clear the span table.
    fn shutdown(&self);
}

/// Zero-effect default `Tracer`. Implements `Tracer` ONLY — it does NOT
/// implement `TracerLifecycle`, since a no-op implementation has nothing to
/// flush or tear down.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoopTracer;

impl Tracer for NoopTracer {
    // Zero-effect: no return, no side effect. The authoritative span identity
    // remains `ctx.span_id()`, re-derived by callers when they end the span.
    fn start_span(&self, _ctx: &TraceContext, _name: &str, _attrs: SpanAttributes) {}

    fn end_span(&self, _span: SpanId, _outcome: SpanOutcome) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    // -----------------------------------------------------------------
    // TASK-001/002: TraceContext::root/from_inbound/child linkage
    // -----------------------------------------------------------------

    #[test]
    fn root_creates_new_trace_with_no_parent() {
        let ctx = TraceContext::root();
        assert_eq!(ctx.parent_span_id(), None);
    }

    #[test]
    fn root_generates_distinct_trace_and_span_ids_each_call() {
        let a = TraceContext::root();
        let b = TraceContext::root();
        assert_ne!(a.trace_id(), b.trace_id());
        assert_ne!(a.span_id(), b.span_id());
    }

    #[test]
    fn from_inbound_creates_new_local_span_with_remote_parent() {
        let remote = TraceContext::root();
        let header = remote.to_traceparent();

        let local = TraceContext::from_inbound(&header).unwrap();

        assert_eq!(local.trace_id(), remote.trace_id());
        assert_eq!(local.parent_span_id(), Some(remote.span_id()));
        assert_ne!(local.span_id(), remote.span_id());
    }

    #[test]
    fn child_links_to_parent_span_same_trace() {
        let parent = TraceContext::root();
        let child = parent.child();

        assert_eq!(child.trace_id(), parent.trace_id());
        assert_eq!(child.parent_span_id(), Some(parent.span_id()));
        assert_ne!(child.span_id(), parent.span_id());
    }

    #[test]
    fn a_to_b_to_c_chain_linkage() {
        // A emits traceparent AAA/111
        let a = TraceContext::root();
        let a_header = a.to_traceparent();

        // B: from_inbound -> same trace, new local span (222), parent = 111
        let b = TraceContext::from_inbound(&a_header).unwrap();
        assert_eq!(b.trace_id(), a.trace_id());
        assert_eq!(b.parent_span_id(), Some(a.span_id()));
        let b_header = b.to_traceparent();

        // C: from_inbound on B's header -> same trace, new span (333), parent = 222
        let c = TraceContext::from_inbound(&b_header).unwrap();
        assert_eq!(c.trace_id(), a.trace_id());
        assert_eq!(c.parent_span_id(), Some(b.span_id()));
        assert_ne!(c.span_id(), a.span_id());
        assert_ne!(c.span_id(), b.span_id());
    }

    // -----------------------------------------------------------------
    // TASK-003/004: W3C traceparent parse/format round-trip
    // -----------------------------------------------------------------

    #[test]
    fn to_traceparent_round_trips_through_parse_traceparent() {
        let ctx = TraceContext::root();
        let header = ctx.to_traceparent();

        let (trace_id, span_id) = parse_traceparent(&header).unwrap();

        assert_eq!(trace_id, ctx.trace_id());
        assert_eq!(span_id, ctx.span_id());
    }

    #[test]
    fn to_traceparent_matches_w3c_shape() {
        let ctx = TraceContext::root();
        let header = ctx.to_traceparent();
        let parts: Vec<&str> = header.split('-').collect();

        assert_eq!(parts.len(), 4);
        assert_eq!(parts[0], "00");
        assert_eq!(parts[1].len(), 32);
        assert_eq!(parts[2].len(), 16);
        assert_eq!(parts[3], "01");
    }

    #[test]
    fn parse_traceparent_raw_decode_only() {
        let ctx = TraceContext::root();
        let header = ctx.to_traceparent();

        let (trace_id, span_id) = parse_traceparent(&header).unwrap();

        // Raw decode only: identical to the ids carried by ctx, but no
        // TraceContext was constructed (no parent linkage semantics here).
        assert_eq!(trace_id, ctx.trace_id());
        assert_eq!(span_id, ctx.span_id());
    }

    #[test]
    fn parse_traceparent_rejects_wrong_field_count() {
        let err = parse_traceparent("00-abc-def").unwrap_err();
        assert_eq!(err, TraceParseError::InvalidFormat);
    }

    #[test]
    fn parse_traceparent_rejects_wrong_length_trace_id() {
        let header = "00-tooshort-00f067aa0ba902b7-01";
        let err = parse_traceparent(header).unwrap_err();
        assert_eq!(err, TraceParseError::InvalidFormat);
    }

    #[test]
    fn parse_traceparent_rejects_non_hex_characters() {
        let header = "00-zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz-00f067aa0ba902b7-01";
        let err = parse_traceparent(header).unwrap_err();
        assert_eq!(err, TraceParseError::InvalidFormat);
    }

    #[test]
    fn from_inbound_rejects_invalid_traceparent() {
        let err = TraceContext::from_inbound("not-a-traceparent").unwrap_err();
        assert_eq!(err, TraceParseError::InvalidFormat);
    }

    // -----------------------------------------------------------------
    // W3C-strict inbound validation (PR1 review): a remote header that
    // W3C forbids must not become a valid EGO trace identity.
    // -----------------------------------------------------------------

    const W3C_VALID: &str = "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01";

    #[test]
    fn parse_traceparent_accepts_valid_v00_header() {
        assert!(parse_traceparent(W3C_VALID).is_ok());
    }

    #[test]
    fn parse_traceparent_rejects_all_zero_trace_id() {
        let header = "00-00000000000000000000000000000000-00f067aa0ba902b7-01";
        assert_eq!(
            parse_traceparent(header).unwrap_err(),
            TraceParseError::InvalidFormat
        );
        // from_inbound relies on the same parse — must also reject.
        assert!(TraceContext::from_inbound(header).is_err());
    }

    #[test]
    fn parse_traceparent_rejects_all_zero_parent_id() {
        let header = "00-4bf92f3577b34da6a3ce929d0e0e4736-0000000000000000-01";
        assert_eq!(
            parse_traceparent(header).unwrap_err(),
            TraceParseError::InvalidFormat
        );
    }

    #[test]
    fn parse_traceparent_rejects_forbidden_version_ff() {
        let header = "ff-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01";
        assert_eq!(
            parse_traceparent(header).unwrap_err(),
            TraceParseError::InvalidFormat
        );
    }

    #[test]
    fn parse_traceparent_accepts_only_version_00() {
        // EGO v1 supports exactly traceparent version `00` — no partial
        // forward-compat for other versions (01, fe, ff) or uppercase (0A/FF).
        for v in ["01", "fe", "ff", "FF", "0a", "0A"] {
            let header = format!("{v}-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01");
            assert_eq!(
                parse_traceparent(&header).unwrap_err(),
                TraceParseError::InvalidFormat,
                "version {v} must be rejected (EGO v1 accepts only version 00)"
            );
        }
        assert!(
            parse_traceparent(W3C_VALID).is_ok(),
            "version 00 must be accepted"
        );
    }

    #[test]
    fn parse_traceparent_rejects_malformed_flags() {
        let header = "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-zz";
        assert_eq!(
            parse_traceparent(header).unwrap_err(),
            TraceParseError::InvalidFormat
        );
    }

    #[test]
    fn parse_traceparent_rejects_uppercase_ids_for_v00() {
        // W3C v1 (version 00) mandates lowercase hex for the id/flags fields.
        let header = "00-4BF92F3577B34DA6A3CE929D0E0E4736-00F067AA0BA902B7-01";
        assert_eq!(
            parse_traceparent(header).unwrap_err(),
            TraceParseError::InvalidFormat
        );
    }

    // -----------------------------------------------------------------
    // TASK-005/006: SpanAttributes allow-list + Tracer + NoopTracer + SpanOutcome
    // -----------------------------------------------------------------

    #[test]
    fn span_attributes_allow_list_only_exposes_safe_scalars() {
        let attrs = SpanAttributes::new()
            .with_tenant_hint_present(true)
            .with_duration(Duration::from_millis(42));

        assert_eq!(attrs.tenant_present(), Some(true));
        assert_eq!(attrs.duration(), Some(Duration::from_millis(42)));

        // Structural (compile-time) guarantee: SpanAttributes has no
        // constructor/field taking a raw tenant id, credential/token, or
        // arbitrary payload. There is no `with_tenant_id`, `with_credential`,
        // or `with_payload`/`with_metadata` method to call here — the type
        // does not expose one.
    }

    #[test]
    fn span_attributes_without_optional_fields() {
        let attrs = SpanAttributes::new();
        assert_eq!(attrs.tenant_present(), None);
        assert_eq!(attrs.duration(), None);
        assert_eq!(attrs.operation_key_hash(), None);
    }

    /// The allow-list carries the *redacted* operation key, and can carry nothing
    /// else about it.
    ///
    /// AD-10 requires `idempotency.operation_key_hash` on the reservation spans.
    /// It is admitted here as one more curated concept, the way
    /// `tenant_present` already is — not as an arbitrary string field, which is
    /// what would actually widen this type's promise.
    ///
    /// The structural point: the builder takes an
    /// [`OperationKeyHash`](crate::operation::OperationKeyHash), whose only
    /// constructor hashes an `OperationKey`. So there is no sequence of calls
    /// that puts a raw client-supplied key on a span — not "no call site does
    /// it", but no such call exists to make. There is deliberately no
    /// `with_operation_key`, and no `String` overload that would accept one.
    #[test]
    fn span_attributes_carry_the_redacted_operation_key_and_not_the_key() {
        use crate::operation::{OperationKey, OperationKeyHash};

        let key = OperationKey::parse("customer-4417-invoice-2026-03").expect("valid key");
        let hash = OperationKeyHash::of(&key);
        let attrs = SpanAttributes::new().with_operation_key_hash(hash.clone());

        assert_eq!(attrs.operation_key_hash(), Some(hash.as_str()));
        assert_eq!(
            attrs.operation_key_hash().map(str::len),
            Some(16),
            "the attribute carries the 16-hex digest, nothing wider"
        );

        // The raw key is absent from every value this type can expose. Asserted
        // over the whole rendered attribute set rather than the one field, so a
        // future field that smuggled the key in would fail here too.
        let rendered = format!("{attrs:?}");
        assert!(
            !rendered.contains("customer-4417-invoice-2026-03"),
            "no attribute may carry the client-supplied key: {rendered}"
        );
    }

    #[test]
    fn noop_tracer_start_span_returns_nothing_authoritative_id_is_ctx_span_id() {
        let tracer = NoopTracer;
        let ctx = TraceContext::root();

        // start_span returns nothing (unit): the authoritative span identity
        // is `TraceContext::span_id()`, which end_span re-derives from `&ctx`.
        // The following line compiling as a statement (no binding, no `-> SpanId`)
        // is itself the proof there is no redundant returned handle.
        tracer.start_span(&ctx, "op.execute", SpanAttributes::new());

        // The id used to end the span comes from the context, not a return value.
        tracer.end_span(ctx.span_id(), SpanOutcome::Ok);
    }

    #[test]
    fn noop_tracer_end_span_is_a_no_op() {
        let tracer = NoopTracer;
        let ctx = TraceContext::root();
        tracer.start_span(&ctx, "op.execute", SpanAttributes::new());
        // The authoritative span id is carried by the context, not returned.
        let span = ctx.span_id();

        // Must not panic; no observable side effect.
        tracer.end_span(span, SpanOutcome::Ok);
        tracer.end_span(
            span,
            SpanOutcome::Error {
                status_message: "boom".to_string(),
            },
        );
    }

    #[test]
    fn span_outcome_error_carries_status_message() {
        let outcome = SpanOutcome::Error {
            status_message: "redacted failure".to_string(),
        };
        match outcome {
            SpanOutcome::Error { status_message } => {
                assert_eq!(status_message, "redacted failure");
            }
            SpanOutcome::Ok => panic!("expected Error outcome"),
        }
    }

    /// Compile-time proof: a plain `Tracer` implementor (NoopTracer) is
    /// usable via a bound requiring `Tracer` only. If `NoopTracer` were
    /// forced to also implement `TracerLifecycle`, that would show up as a
    /// design defect, not as a failure of this generic bound — but the
    /// absence of any `impl TracerLifecycle for NoopTracer` block above is
    /// what actually proves NoopTracer compiles without it.
    #[test]
    fn noop_tracer_satisfies_tracer_bound_without_lifecycle() {
        fn assert_is_tracer<T: Tracer>(_t: &T) {}
        let tracer = NoopTracer;
        assert_is_tracer(&tracer);
    }
}
