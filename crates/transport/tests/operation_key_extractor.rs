//! PROD-012 B6.5 — the HTTP end of the `OperationKey` extraction contract.
//!
//! Three pieces existed before this and nothing joined them: the header
//! carrier, the shared policy table, and the runtime's configured mode. So the
//! guarantee the SDK enforces internally could be disabled simply by deploying
//! a transport that never asked. These tests are about the join.
//!
//! Every case runs the real extractor against a real `AppState` built from a
//! real `RuntimeBuilder`, so what is exercised is the wiring rather than a
//! restatement of `resolve_operation_key`'s own unit tests.

use std::sync::Arc;

use axum::extract::FromRequestParts;
use axum::http::Request;
use ego_domain::MetricKind;
use ego_security_sdk::authentication::AuthenticationProvider;
use ego_service_sdk::runtime::{IdempotencyEnforcementMode, RuntimeBuilder};
use ego_transport::state::AppState;
use ego_transport::{OperationKeyExtractor, TransportError};

/// The extractor never authenticates, so the provider only has to exist.
struct UnusedAuthn;
impl AuthenticationProvider for UnusedAuthn {
    fn authenticate(
        &self,
        _c: &ego_security_sdk::credential::Credential,
    ) -> Result<ego_security_sdk::context::SecurityContext, ego_security_sdk::AuthenticationError>
    {
        unimplemented!("the operation-key boundary does not authenticate");
    }
}

/// A state whose runtime was built under `mode` — and only under `mode`. The
/// extractor has no configuration of its own, so this is the single input that
/// decides its behaviour.
fn state_under(mode: IdempotencyEnforcementMode) -> AppState {
    let mut builder = RuntimeBuilder::new().with_idempotency_enforcement_mode(mode);
    if matches!(mode, IdempotencyEnforcementMode::MandatoryKey) {
        // The builder refuses `MandatoryKey` with nowhere to reserve — a runtime
        // that promises every mutating operation carries a key and cannot
        // reserve one could not keep the promise. Registering a store is what
        // makes the promise buildable, not what makes the header required.
        builder = builder
            .with_operation_reservation_store(Arc::new(
                ego_testkit::InMemoryOperationReservationStore::new(Arc::new(
                    ego_domain::time::SystemClock,
                )),
            ))
            .with_reservation_owner_id(ego_domain::operation::OwnerId::new("under-test"))
            .with_reservation_lease_duration(std::time::Duration::from_secs(30));
    }
    AppState::new(builder.build().resolver(), Arc::new(UnusedAuthn))
}

/// Runs the real extractor over a request carrying `header`, if any.
async fn extract(
    mode: IdempotencyEnforcementMode,
    header: Option<&str>,
) -> Result<OperationKeyExtractor, TransportError> {
    let mut request = Request::builder().uri("/register");
    if let Some(value) = header {
        request = request.header("Idempotency-Key", value);
    }
    let (mut parts, _) = request.body(()).expect("a valid request").into_parts();
    let state = state_under(mode);
    OperationKeyExtractor::from_request_parts(&mut parts, &state).await
}

// ---------------------------------------------------------------------------
// The mode the extractor applies is the one the runtime was built under
// ---------------------------------------------------------------------------

/// The runtime reports the policy it was built under, whichever it is. The
/// whole boundary reads this value, so a builder that validated one policy and
/// a runtime that reported another would let a deployment enforce something
/// nobody configured.
///
/// **The default is the fail-closed one.** `MandatoryKey`, not `Compatibility`
/// — a caller who never thought about idempotency must not silently get none.
/// Pinned here because the extractor's behaviour on an unconfigured deployment
/// follows from it, and because it is the opposite of what "default" usually
/// suggests.
#[test]
fn the_runtime_reports_the_mode_it_was_built_under() {
    assert_eq!(
        IdempotencyEnforcementMode::default(),
        IdempotencyEnforcementMode::MandatoryKey,
        "the default fails closed; a runtime that reported otherwise would admit \
         keyless requests nobody agreed to admit"
    );

    // Built under the default, which is only buildable with somewhere to
    // reserve — the builder refuses the combination that could not keep its
    // promise, and that refusal is why this state needs a store at all.
    assert_eq!(
        state_under(IdempotencyEnforcementMode::MandatoryKey)
            .runtime
            .idempotency_enforcement_mode(),
        IdempotencyEnforcementMode::MandatoryKey
    );

    assert_eq!(
        state_under(IdempotencyEnforcementMode::Compatibility)
            .runtime
            .idempotency_enforcement_mode(),
        IdempotencyEnforcementMode::Compatibility,
        "an explicitly permissive deployment is reported as permissive, not \
         normalised toward the default"
    );
}

// ---------------------------------------------------------------------------
// A key that arrives is carried, unchanged
// ---------------------------------------------------------------------------

/// The value the client sent is the value that reaches the context — parsed,
/// never regenerated, normalised or reconstructed. This is the assertion a
/// handler that minted its own key would fail.
#[tokio::test]
async fn a_valid_header_is_carried_through_unchanged() {
    for mode in [
        IdempotencyEnforcementMode::Compatibility,
        IdempotencyEnforcementMode::MandatoryKey,
    ] {
        let OperationKeyExtractor(key) = extract(mode, Some("op-from-the-client"))
            .await
            .expect("a well-formed key is admitted under every mode");

        assert_eq!(
            key.expect("a present key resolves to Some").as_str(),
            "op-from-the-client",
            "{mode:?}: the client's key, byte for byte"
        );
    }
}

/// HTTP/2 lowercases every header name on the wire, so this is what a real
/// client sends. Covered at the carrier already; repeated here because the
/// wiring is what would drop it.
#[tokio::test]
async fn the_header_is_found_in_the_case_a_real_client_sends() {
    let mut request = Request::builder().uri("/register");
    request = request.header("idempotency-key", "op-lowercased");
    let (mut parts, _) = request.body(()).expect("a valid request").into_parts();
    let state = state_under(IdempotencyEnforcementMode::MandatoryKey);

    let OperationKeyExtractor(key) = OperationKeyExtractor::from_request_parts(&mut parts, &state)
        .await
        .expect("a lowercased header name is the same header");

    assert_eq!(key.expect("resolves").as_str(), "op-lowercased");
}

// ---------------------------------------------------------------------------
// What each mode does about a key that never arrived
// ---------------------------------------------------------------------------

/// The point of the whole boundary. Under the enforcing mode a request with no
/// key is refused **here** — the handler is never reached, so the operation
/// cannot run unreserved.
#[tokio::test]
async fn a_missing_key_is_refused_before_the_operation_under_mandatory_key() {
    let result = extract(IdempotencyEnforcementMode::MandatoryKey, None).await;

    assert!(
        matches!(result, Err(TransportError::BadRequest)),
        "a runtime that requires a key must not dispatch a request without one"
    );
}

/// And the mirror: a deployment that declared itself still in transition
/// dispatches normally. `None` is the answer, not an error — the request simply
/// carries no operation identity.
#[tokio::test]
async fn a_missing_key_is_admitted_under_compatibility() {
    let OperationKeyExtractor(key) = extract(IdempotencyEnforcementMode::Compatibility, None)
        .await
        .expect("compatibility exists precisely to admit this");

    assert!(key.is_none());
}

// ---------------------------------------------------------------------------
// Compatibility tolerates absence — never malformed input
// ---------------------------------------------------------------------------

/// The distinction the whole policy rests on. `Compatibility` admits a request
/// that carried *no* key; it does not admit one that carried an unusable one.
/// Collapsing the two would let malformed input silently disable the guarantee
/// for exactly the requests most likely to come from a broken client.
#[tokio::test]
async fn an_invalid_key_is_refused_under_both_modes() {
    for mode in [
        IdempotencyEnforcementMode::Compatibility,
        IdempotencyEnforcementMode::MandatoryKey,
    ] {
        let result = extract(mode, Some("   ")).await;

        assert!(
            matches!(result, Err(TransportError::BadRequest)),
            "{mode:?}: a whitespace-only key is supplied-but-unusable, which no \
             mode admits — compatibility tolerates absence, not garbage"
        );
    }
}

// ---------------------------------------------------------------------------
// Counting refused idempotency keys, by reason
// ---------------------------------------------------------------------------

use ego_testkit::RecordedMetric;

/// Records every `metric` call in order.
#[derive(Default)]
struct RecordingObservability {
    metrics: std::sync::Mutex<Vec<RecordedMetric>>,
}

impl RecordingObservability {
    fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }
    /// Whole records, so `reason` and `carrier` are compared and not just the name.
    fn records(&self) -> Vec<RecordedMetric> {
        self.metrics.lock().expect("not poisoned").clone()
    }
    fn names(&self) -> Vec<String> {
        self.metrics
            .lock()
            .expect("not poisoned")
            .iter()
            .map(|m| m.name.clone())
            .collect()
    }
}

impl ego_domain::Observability for RecordingObservability {
    fn trace(&self, _e: ego_domain::SemanticEvent) {}
    fn record_metric(&self, observation: ego_domain::MetricObservation<'_>) {
        self.metrics
            .lock()
            .expect("not poisoned")
            .push(RecordedMetric::capture(&observation));
    }
    fn log(&self, _l: ego_domain::Level, _m: &str) {}
}

/// Same wiring as `state_under`, plus a recording `Observability`.
fn instrumented_state_under(
    mode: IdempotencyEnforcementMode,
    obs: Arc<RecordingObservability>,
) -> AppState {
    let mut builder = RuntimeBuilder::new()
        .with_idempotency_enforcement_mode(mode)
        .with_observability(obs as Arc<dyn ego_domain::Observability>);
    if matches!(mode, IdempotencyEnforcementMode::MandatoryKey) {
        builder = builder
            .with_operation_reservation_store(Arc::new(
                ego_testkit::InMemoryOperationReservationStore::new(Arc::new(
                    ego_domain::time::SystemClock,
                )),
            ))
            .with_reservation_owner_id(ego_domain::operation::OwnerId::new("under-test"))
            .with_reservation_lease_duration(std::time::Duration::from_secs(30));
    }
    AppState::new(builder.build().resolver(), Arc::new(UnusedAuthn))
}

/// Runs the real extractor against an instrumented runtime and returns what was
/// counted alongside the outcome.
async fn extract_instrumented(
    mode: IdempotencyEnforcementMode,
    header: Option<&str>,
) -> (
    Result<OperationKeyExtractor, TransportError>,
    Arc<RecordingObservability>,
) {
    let obs = RecordingObservability::new();
    let mut request = Request::builder().uri("/register");
    if let Some(value) = header {
        request = request.header("Idempotency-Key", value);
    }
    let (mut parts, _) = request.body(()).expect("a valid request").into_parts();
    let state = instrumented_state_under(mode, obs.clone());
    let result = OperationKeyExtractor::from_request_parts(&mut parts, &state).await;
    (result, obs)
}

/// The carrier every rejection here reports.
///
/// Written out as a literal **on purpose**, not for want of an accessor. Asking
/// `HeaderCarrier::carrier_name()` for it would rename both sides of the assertion
/// at once, so renaming the location this transport reports would pass silently.
/// A literal makes that rename fail here, which is what forces a deliberate change
/// to be acknowledged rather than absorbed.
///
/// It is not the guard against the *wiring* going wrong — that the dimension comes
/// from the rejection rather than being re-derived is pinned separately, by a
/// mutation that replaces it with a plausible constant.
const HTTP_CARRIER: &str = "http:Idempotency-Key";

/// One emission, whole: every field AD-10 fixes for this signal.
///
/// The kind is included deliberately. Projecting only name, value and dimensions
/// leaves `counter` → `gauge` undetectable — same name, same `1.0`, same
/// attributes — which would leave one row of AD-10 partly unproven in the very
/// slice that moved it onto the typed port.
fn rejection_shape(m: &RecordedMetric) -> (MetricKind, String, f64, Vec<(String, String)>) {
    (m.kind, m.name.clone(), m.value, m.attributes.clone())
}

/// The single record a rejection must produce, for a given reason.
///
/// Spelled out whole so an assertion cannot pass with any one part right and
/// another missing — a call site left on the pre-migration shape, a dimension
/// dropped, or the wrong aggregation exported.
fn expected_rejection(reason: &str) -> (MetricKind, String, f64, Vec<(String, String)>) {
    (
        MetricKind::Counter,
        "idempotency.key.rejected".to_string(),
        1.0,
        vec![
            ("reason".to_string(), reason.to_string()),
            ("carrier".to_string(), HTTP_CARRIER.to_string()),
        ],
    )
}

// ---------------------------------------------------------------------------
// Issue #306 — a panicking metric sink must not take the request down with it
// ---------------------------------------------------------------------------

/// `record_metric` always panics; `trace`/`log` are unused no-ops. Stands in
/// for a broken metrics backend wired into a real deployment's runtime.
#[derive(Default)]
struct PanickingObservability;

impl ego_domain::Observability for PanickingObservability {
    fn trace(&self, _e: ego_domain::SemanticEvent) {}
    fn record_metric(&self, _observation: ego_domain::MetricObservation<'_>) {
        panic!("PanickingObservability::record_metric always panics (test double)");
    }
    fn log(&self, _l: ego_domain::Level, _m: &str) {}
}

/// Same wiring as `instrumented_state_under`, with a sink that panics on every
/// metric emission instead of recording it.
fn state_with_panicking_observability(mode: IdempotencyEnforcementMode) -> AppState {
    let mut builder = RuntimeBuilder::new()
        .with_idempotency_enforcement_mode(mode)
        .with_observability(Arc::new(PanickingObservability) as Arc<dyn ego_domain::Observability>);
    if matches!(mode, IdempotencyEnforcementMode::MandatoryKey) {
        builder = builder
            .with_operation_reservation_store(Arc::new(
                ego_testkit::InMemoryOperationReservationStore::new(Arc::new(
                    ego_domain::time::SystemClock,
                )),
            ))
            .with_reservation_owner_id(ego_domain::operation::OwnerId::new("under-test"))
            .with_reservation_lease_duration(std::time::Duration::from_secs(30));
    }
    AppState::new(builder.build().resolver(), Arc::new(UnusedAuthn))
}

/// The exact scenario issue #306 names: "for the transport-edge counters it is
/// a request." A missing key under `MandatoryKey` reaches the same
/// `obs.counter("idempotency.key.rejected", ...)` call `a_missing_key_counts_
/// the_missing_rejection` above exercises against a recording sink — here the
/// sink panics on that call instead, and the request must still resolve to the
/// same `400 BadRequest` a healthy sink would produce. `result.is_err()` alone
/// would not distinguish a real rejection from a panic that unwound through
/// the extractor and was reported as some other failure — the `matches!` below
/// pins it to the exact variant a healthy sink produces.
#[tokio::test]
async fn a_panicking_metric_sink_does_not_fail_the_request_it_is_only_counting() {
    let state = state_with_panicking_observability(IdempotencyEnforcementMode::MandatoryKey);
    let (mut parts, _) = Request::builder()
        .uri("/register")
        .body(())
        .expect("a valid request")
        .into_parts();

    let result = OperationKeyExtractor::from_request_parts(&mut parts, &state).await;

    assert!(
        matches!(result, Err(TransportError::BadRequest)),
        "a missing key must still be refused with the same 400 a healthy \
         metrics sink would produce, not silently turned into something else \
         by the panic"
    );
}

/// A missing key under `MandatoryKey` counts one rejection, once.
#[tokio::test]
async fn a_missing_key_counts_the_missing_rejection() {
    let (result, obs) = extract_instrumented(IdempotencyEnforcementMode::MandatoryKey, None).await;
    assert!(
        result.is_err(),
        "a missing key is refused under MandatoryKey"
    );
    assert_eq!(
        obs.records()
            .iter()
            .map(rejection_shape)
            .collect::<Vec<_>>(),
        vec![expected_rejection("missing")],
        "exactly one counter, carrying the reason and the carrier that reported it"
    );
}

/// A malformed key counts `…rejected.invalid`, under either mode.
///
/// Both modes are exercised because `Compatibility` loosens only the *missing*-key
/// policy: an invalid key is refused either way, so it must be counted either way. A
/// counter wired only into the mandatory path would go quiet exactly where a
/// deployment had loosened the rules and most needed to see the traffic.
#[tokio::test]
async fn a_malformed_key_counts_the_invalid_rejection_under_both_modes() {
    for mode in [
        IdempotencyEnforcementMode::MandatoryKey,
        IdempotencyEnforcementMode::Compatibility,
    ] {
        let (result, obs) = extract_instrumented(mode, Some("   ")).await;
        assert!(result.is_err(), "{mode:?} refuses a whitespace-only key");
        assert_eq!(
            obs.records()
                .iter()
                .map(rejection_shape)
                .collect::<Vec<_>>(),
            vec![expected_rejection("invalid")],
            "{mode:?} must count the invalid rejection"
        );
    }
}

/// Runs the real extractor against a header carrying bytes that are not text.
///
/// Separate from `extract_instrumented` because that helper takes a `&str`, and a value
/// that never became a string is the whole point here.
async fn extract_instrumented_raw_header(
    mode: IdempotencyEnforcementMode,
    bytes: &[u8],
) -> (
    Result<OperationKeyExtractor, TransportError>,
    Arc<RecordingObservability>,
) {
    let obs = RecordingObservability::new();
    let request = Request::builder().uri("/register").header(
        "Idempotency-Key",
        axum::http::HeaderValue::from_bytes(bytes).expect("a byte header value"),
    );
    let (mut parts, _) = request.body(()).expect("a valid request").into_parts();
    let state = instrumented_state_under(mode, obs.clone());
    let result = OperationKeyExtractor::from_request_parts(&mut parts, &state).await;
    (result, obs)
}

/// A header whose bytes are not text counts `…rejected.unreadable`, under either mode.
///
/// The third name, and it exists because `OperationKeyRejection` keeps `Unreadable`
/// apart from `Invalid` on purpose: no `OperationKeyError` describes a value that never
/// became a string. Folding them would discard exactly that distinction — an operator
/// seeing `unreadable` is looking at a transport or encoding fault, while `invalid` is a
/// client sending a malformed key, and the two lead somewhere different.
///
/// Both modes are exercised for the same reason the invalid case is: `Compatibility`
/// loosens only the *missing*-key policy, so a value that never became a string is
/// refused either way and must be counted either way.
///
/// Reachable, not theoretical: `HeaderValue` holds arbitrary bytes and `to_str` fails on
/// non-UTF-8, which is what `HeaderCarrier` reports as `Unreadable`.
#[tokio::test]
async fn a_header_that_is_not_text_counts_the_unreadable_rejection_under_both_modes() {
    for mode in [
        IdempotencyEnforcementMode::MandatoryKey,
        IdempotencyEnforcementMode::Compatibility,
    ] {
        let (result, obs) = extract_instrumented_raw_header(mode, &[0xff, 0xfe]).await;
        assert!(result.is_err(), "{mode:?} refuses a key that is not text");
        assert_eq!(
            obs.records()
                .iter()
                .map(rejection_shape)
                .collect::<Vec<_>>(),
            vec![expected_rejection("unreadable")],
            "{mode:?}: an unreadable value is its own reason, not an invalid string"
        );
    }
}

/// An accepted request counts nothing.
///
/// The negative control, and it covers both ways a request is accepted: a valid key,
/// and a *missing* key under `Compatibility` — which is an admission, not a rejection,
/// and must not be counted as one. Without the second case a counter emitted on every
/// extraction would pass.
#[tokio::test]
async fn an_accepted_request_counts_nothing() {
    let (result, obs) =
        extract_instrumented(IdempotencyEnforcementMode::MandatoryKey, Some("op-1")).await;
    assert!(result.is_ok(), "a valid key is carried through");
    assert!(
        obs.names().is_empty(),
        "nothing was rejected: {:?}",
        obs.names()
    );

    let (result, obs) = extract_instrumented(IdempotencyEnforcementMode::Compatibility, None).await;
    assert!(result.is_ok(), "Compatibility admits a missing key");
    assert!(
        obs.names().is_empty(),
        "an admission is not a rejection: {:?}",
        obs.names()
    );
}

/// This file's double preserves the dimensions it is handed.
///
/// The extractor's counters carry none today, so nothing else here would notice
/// if the double dropped them — and a later signal that does carry dimensions
/// would lose them silently.
#[test]
fn the_double_preserves_metric_observations() {
    let obs = RecordingObservability::new();
    ego_testkit::assert_metric_observations_are_preserved(obs.as_ref(), || {
        obs.metrics.lock().expect("not poisoned").clone()
    });
}

/// An uninstrumented runtime rejects and admits exactly as before.
#[tokio::test]
async fn an_uninstrumented_extractor_behaves_identically() {
    assert!(extract(IdempotencyEnforcementMode::MandatoryKey, None)
        .await
        .is_err());
    assert!(
        extract(IdempotencyEnforcementMode::MandatoryKey, Some("op-1"))
            .await
            .is_ok()
    );
    assert!(extract(IdempotencyEnforcementMode::Compatibility, None)
        .await
        .is_ok());
}

// ---------------------------------------------------------------------------
// AD-10 redaction: the raw key reaches no dimension
// ---------------------------------------------------------------------------

/// A value no other string in this file could produce by accident.
///
/// Shaped like something a client would really send — a business identifier — so
/// the scan below is looking for the kind of value the redaction rule exists to
/// keep out, not for a placeholder.
const CANARY: &str = "customer-4417-invoice-2026-03-canary";

/// A key that carries the canary **and** is genuinely refused.
///
/// `OperationKey::parse` rejects exactly two shapes: empty-after-trim, and longer
/// than the maximum. Only the second can also carry a recognisable value, so the
/// canary is padded past the limit — a key that is rejected *while containing the
/// thing that must not leak* is the only arrangement that makes the scan mean
/// anything. Trimmed padding would simply be accepted, and the test would assert
/// redaction on a request that produced no rejection at all.
fn over_long_key_carrying_the_canary() -> String {
    format!("{CANARY}{}", "x".repeat(4096))
}

/// The client's key appears in no metric this boundary emits.
///
/// Scanned across the whole record rather than one field: the name, the value, and
/// **both halves of every attribute**. A key smuggled in as a dimension *key* is as
/// unbounded as one smuggled in as a dimension value, and a check that only read
/// values would miss it.
///
/// All three reasons are covered. `unreadable` matters most: it is the one that
/// never produced an `OperationKeyError`, so it is the arm where a well-meaning
/// "include the offending value for diagnostics" is most tempting and least
/// constrained.
///
/// Since the typed-metric port landed, this is a rule to keep and test rather than
/// something the types make unrepresentable — the port now carries attributes, so
/// nothing structurally prevents the key from becoming one.
#[tokio::test]
async fn no_metric_carries_the_raw_operation_key() {
    // `invalid`: a key that parses as text and fails validation, carrying the canary.
    let (result, obs) = extract_instrumented(
        IdempotencyEnforcementMode::MandatoryKey,
        Some(&over_long_key_carrying_the_canary()),
    )
    .await;
    assert!(result.is_err(), "an over-long key is refused");

    let recorded = obs.records();
    assert!(
        !recorded.is_empty(),
        "the scan is only meaningful if something was emitted"
    );
    for m in &recorded {
        let rendered = format!("{m:?}");
        assert!(
            !rendered.contains(CANARY),
            "the client-supplied key must reach no name, no value, and neither half \
             of any dimension: {rendered}"
        );
    }
}

/// And the same for a value that never became a string.
#[tokio::test]
async fn an_unreadable_value_reaches_no_dimension_either() {
    let mut bytes = CANARY.as_bytes().to_vec();
    // Not UTF-8, so the carrier reports `Unreadable` — while the bytes still
    // contain the canary, which is what makes the scan meaningful.
    bytes.push(0xff);

    let (result, obs) =
        extract_instrumented_raw_header(IdempotencyEnforcementMode::MandatoryKey, &bytes).await;
    assert!(result.is_err(), "a value that is not text is refused");

    let recorded = obs.records();
    assert_eq!(
        recorded.iter().map(rejection_shape).collect::<Vec<_>>(),
        vec![expected_rejection("unreadable")],
        "only the reason and the carrier are reported"
    );
    for m in &recorded {
        let rendered = format!("{m:?}");
        assert!(
            !rendered.contains(CANARY),
            "an unreadable value is still client input and reaches no dimension: {rendered}"
        );
    }
}

/// The carrier names a location, never what was found there.
///
/// This is the property that makes `carrier` admissible as a dimension at all: it
/// is drawn from a fixed set of transport locations, so its cardinality is bounded
/// by how many carriers exist rather than by how many requests arrive.
#[tokio::test]
async fn the_carrier_names_the_location_and_not_the_value() {
    let (_, obs) = extract_instrumented(
        IdempotencyEnforcementMode::MandatoryKey,
        Some(&over_long_key_carrying_the_canary()),
    )
    .await;

    let carriers: Vec<String> = obs
        .records()
        .iter()
        .flat_map(|m| m.attributes.clone())
        .filter(|(k, _)| k == "carrier")
        .map(|(_, v)| v)
        .collect();

    assert_eq!(
        carriers,
        vec![HTTP_CARRIER.to_string()],
        "the carrier is the stable name of the header consulted, not its contents"
    );
}

/// No folded name survives the migration.
///
/// The reason used to be encoded in the name, so this guards a call site left
/// behind — which the positive assertions cannot see, since they compare what was
/// emitted rather than what must never be.
#[tokio::test]
async fn no_rejection_emits_a_folded_name() {
    let cases = [
        (IdempotencyEnforcementMode::MandatoryKey, None),
        (IdempotencyEnforcementMode::MandatoryKey, Some("   ")),
        (IdempotencyEnforcementMode::Compatibility, Some("   ")),
    ];
    for (mode, header) in cases {
        let (_, obs) = extract_instrumented(mode, header).await;
        let names = obs.names();
        assert!(
            names.iter().all(|n| n == "idempotency.key.rejected"),
            "{mode:?} with {header:?}: the reason belongs in a dimension, so no name \
             may carry it: {names:?}"
        );
    }
}
