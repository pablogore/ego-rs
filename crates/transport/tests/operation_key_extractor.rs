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
    fn names(&self) -> Vec<String> {
        self.metrics
            .lock()
            .expect("not poisoned")
            .iter()
            .map(|m| m.name.clone())
            .collect()
    }
    fn values(&self) -> Vec<f64> {
        self.metrics
            .lock()
            .expect("not poisoned")
            .iter()
            .map(|m| m.value)
            .collect()
    }
    fn last_metric_attributes(&self) -> Vec<(String, String)> {
        self.metrics
            .lock()
            .expect("not poisoned")
            .last()
            .map(|m| m.attributes.clone())
            .unwrap_or_default()
    }
}

impl ego_domain::Observability for RecordingObservability {
    fn trace(&self, _e: ego_domain::SemanticEvent) {}
    fn metric_with_attributes(
        &self,
        name: &'static str,
        value: f64,
        attributes: &[ego_domain::MetricAttribute<'_>],
    ) {
        self.metrics
            .lock()
            .expect("not poisoned")
            .push(RecordedMetric::capture(name, value, attributes));
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

/// A missing key under `MandatoryKey` counts `…rejected.missing`, once.
#[tokio::test]
async fn a_missing_key_counts_the_missing_rejection() {
    let (result, obs) = extract_instrumented(IdempotencyEnforcementMode::MandatoryKey, None).await;
    assert!(
        result.is_err(),
        "a missing key is refused under MandatoryKey"
    );
    assert_eq!(
        obs.names(),
        vec!["idempotency.key.rejected.missing".to_string()],
        "exactly one counter, naming the reason"
    );
    assert_eq!(obs.values(), vec![1.0], "a counter increment is one");
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
            obs.names(),
            vec!["idempotency.key.rejected.invalid".to_string()],
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
            obs.names(),
            vec!["idempotency.key.rejected.unreadable".to_string()],
            "{mode:?}: an unreadable value is its own reason, not an invalid string"
        );
        assert_eq!(
            obs.values(),
            vec![1.0],
            "{mode:?}: a counter increment is one"
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
fn the_double_preserves_metric_attributes() {
    let obs = RecordingObservability::new();
    ego_testkit::assert_metric_attributes_are_preserved(obs.as_ref(), || {
        obs.last_metric_attributes()
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
