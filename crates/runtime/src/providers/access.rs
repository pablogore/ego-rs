//! `RuntimeDataProviderAccess` — the runtime-side implementation of
//! `persistent_entity::data_provider_access::DataProviderAccess` (CORE-019A
//! Phase 3, AD-003/AD-008/AD-009). The sole runtime implementation of the
//! handler-facing port: performs the registry lookup and is the single
//! observability chokepoint. Every fetch attempt — success, fail-closed
//! miss, or provider-reported error — emits exactly one `tracing` event
//! carrying `provider_id`, a hashed `key` (never the raw key), `latency`,
//! `cache_hit`, and an explicit `outcome: ProviderOutcome` derived once here
//! (AD-008). `payload` is never logged, and `DataProviderError::Transient`/
//! `Fatal`'s free-text message is never logged either — only the outcome
//! classification (PR1 review G-02 watch-item, design.md §11).
//!
//! Mirrors `crate::effects::observability`'s `hashed_key` + `log_*`
//! convention (named function wrapping a `tracing` macro, called from the
//! actual chokepoint rather than inlining `tracing::info!` there). Issue #234
//! reconciled the two by making this module call
//! `crate::effects::observability::hashed_key` directly instead of keeping its
//! own byte-identical copy.
//!
//! # Issue #234 hardening (timeout + retry)
//!
//! This chokepoint is also the single place provider I/O gets a uniform
//! timeout and uniform retry orchestration (both configurable via
//! [`ProviderAccessConfig`]), so no individual provider re-implements either
//! and no adapter invents its own transient/fatal classification:
//!
//! * **Timeout** — each attempt runs under `tokio::time::timeout` (the runtime
//!   already uses `tokio::time` directly throughout `crate::effects::runner`;
//!   there is no clock abstraction to route through, and the handler-facing
//!   SPI in `persistent_entity` stays Tokio-free). Elapsing yields
//!   [`DataProviderError::Timeout`], distinct from a provider's own errors,
//!   which are never lost. Dropping the timed-out future cancels the in-flight
//!   provider call, preserving async cancellation.
//! * **Retry** — reuses the effects [`RetryPolicy`] (bounded, jittered,
//!   exponential backoff) rather than a second retry model. Only `Transient`
//!   and `Timeout` are retried; `Fatal`, `NotFound`, and `ProviderMissing` are
//!   never retried. The attempt count is bounded by the policy, so there is no
//!   unbounded retry loop.

use std::time::{Duration, Instant};

use async_trait::async_trait;
use persistent_entity::data_provider_access::{
    DataProviderAccess, DataProviderError, DataRequest, DataResponse,
};
use tracing::{info, info_span, Instrument};

use crate::effects::observability::hashed_key;
use crate::effects::RetryPolicy;

use super::registry::ExternalDataProviderRegistry;

/// AD-234 default: the uniform per-attempt timeout applied to every provider
/// fetch. Deliberately generous so it does not surprise a legitimately slow
/// provider that predates this change; tune it down via
/// [`ProviderAccessConfig`].
pub const DEFAULT_PROVIDER_TIMEOUT: Duration = Duration::from_secs(30);

/// Cross-cutting policy applied uniformly at the access chokepoint (issue
/// #234): the per-attempt I/O timeout and the retry policy for transient
/// failures. Reuses the effects [`RetryPolicy`] verbatim rather than defining
/// a second, incompatible backoff model.
///
/// **Behavior change (issue #234):** [`ProviderAccessConfig::default`] enables
/// retries (`RetryPolicy::default` = 3 retries). Before #234 a `Transient`
/// failure returned immediately; now every fetch under the default config
/// retries `Transient`/`Timeout` up to 3 times with backoff. Because the
/// timeout is *per attempt* (not a whole-call deadline), the worst-case
/// latency of a failing fetch grows to roughly `(max_attempts + 1) * timeout`
/// plus the summed backoff. This is API-compatible but not behavior-identical;
/// pass a config with [`RetryPolicy::none`] to restore the pre-#234
/// return-immediately behavior.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProviderAccessConfig {
    /// Per-attempt timeout. Each individual attempt (initial or retry) is
    /// bounded by this; it is not a deadline across all attempts.
    pub timeout: Duration,
    /// Retry policy for retryable failures (`Transient` and `Timeout`). Its
    /// `max_attempts` bounds the number of retries — total attempts are
    /// `max_attempts + 1`. Defaults (via [`ProviderAccessConfig::default`]) to
    /// [`RetryPolicy::default`] (3 retries) — see the type-level note on the
    /// resulting behavior change. Use [`RetryPolicy::none`] to disable retries.
    pub retry: RetryPolicy,
}

impl Default for ProviderAccessConfig {
    fn default() -> Self {
        Self {
            timeout: DEFAULT_PROVIDER_TIMEOUT,
            retry: RetryPolicy::default(),
        }
    }
}

/// Fetch outcome classification, derived once at the chokepoint from a
/// `Result<DataResponse, DataProviderError>` (AD-008) — queryable/alertable
/// without parsing an error's free-text message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderOutcome {
    /// The fetch completed successfully.
    Success,
    /// The provider was resolved but had no data for the requested key.
    NotFound,
    /// A retryable provider failure.
    Transient,
    /// The attempt exceeded the configured timeout (issue #234) — retryable,
    /// distinct from `Transient`.
    Timeout,
    /// A non-retryable provider failure.
    Fatal,
    /// No provider is registered for the requested `provider_id`
    /// (fail-closed resolution).
    ProviderMissing,
}

impl ProviderOutcome {
    fn as_str(self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::NotFound => "not_found",
            Self::Transient => "transient",
            Self::Timeout => "timeout",
            Self::Fatal => "fatal",
            Self::ProviderMissing => "provider_missing",
        }
    }

    fn from_result(result: &Result<DataResponse, DataProviderError>) -> Self {
        match result {
            Ok(_) => Self::Success,
            Err(DataProviderError::NotFound { .. }) => Self::NotFound,
            Err(DataProviderError::Transient(_)) => Self::Transient,
            Err(DataProviderError::Timeout) => Self::Timeout,
            Err(DataProviderError::Fatal(_)) => Self::Fatal,
            Err(DataProviderError::ProviderMissing { .. }) => Self::ProviderMissing,
        }
    }

    /// Whether a fetch with this outcome may be retried (issue #234). Only
    /// `Transient` and `Timeout` are retryable; `Fatal`/`NotFound` are
    /// definitive answers and `ProviderMissing` is a bootstrap error — none of
    /// those are ever retried.
    fn is_retryable(self) -> bool {
        matches!(self, Self::Transient | Self::Timeout)
    }
}

/// The exact field values the terminal `data_provider_fetch` signal carries.
///
/// Built by the pure [`fetch_signal`] so redaction (`key_hash`, never the raw
/// key) and classification (`outcome`) can be asserted directly in tests,
/// without capturing through `tracing`'s process-wide per-callsite interest
/// cache — the CORE-027 flaky-capture race documented in
/// `crate::effects::observability`, which that module resolved the same way.
/// Structurally, this type has no field able to carry a
/// `DataProviderError::Transient`/`Fatal` message string or the `payload`: the
/// only inputs are the already-classified [`ProviderOutcome`] and the key
/// (used solely to hash), so a message/payload leak is impossible by
/// construction, not merely absent.
#[derive(Debug, Clone, PartialEq, Eq)]
struct FetchSignal {
    provider_id: String,
    key_hash: String,
    latency_ms: u64,
    cache_hit: bool,
    outcome: &'static str,
    /// Total attempt count (issue #234) — `1` when no retry happened. A
    /// low-cardinality counter dimension.
    attempts: u32,
}

fn fetch_signal(
    provider_id: &str,
    key: &str,
    latency: Duration,
    cache_hit: bool,
    outcome: ProviderOutcome,
    attempts: u32,
) -> FetchSignal {
    FetchSignal {
        provider_id: provider_id.to_string(),
        key_hash: hashed_key(key),
        latency_ms: latency.as_millis() as u64,
        cache_hit,
        outcome: outcome.as_str(),
        attempts,
    }
}

/// Emits the one terminal observability event a `fetch` call produces (AD-008,
/// spec: "Fetch Observability Signals"), regardless of how many attempts it
/// took. Thin wiring over [`fetch_signal`]: the `info!` macro fixes the field
/// names/values at this callsite at compile time, so correctness of the field
/// *values* is asserted on `fetch_signal` directly, not through this call.
fn log_fetch(
    provider_id: &str,
    key: &str,
    latency: Duration,
    cache_hit: bool,
    outcome: ProviderOutcome,
    attempts: u32,
) {
    let f = fetch_signal(provider_id, key, latency, cache_hit, outcome, attempts);
    info!(
        provider_id = %f.provider_id,
        key_hash = %f.key_hash,
        latency_ms = f.latency_ms,
        cache_hit = f.cache_hit,
        outcome = f.outcome,
        attempts = f.attempts,
        "data_provider_fetch"
    );
}

/// The exact field values the per-retry `data_provider_fetch_retry` signal
/// carries (issue #234) — mirrors
/// `crate::effects::observability::log_retry_scheduled`. Like [`FetchSignal`],
/// it can carry only the classification and low-cardinality retry dimensions;
/// there is no field for the key or any error message text.
#[derive(Debug, Clone, PartialEq, Eq)]
struct RetrySignal {
    provider_id: String,
    key_hash: String,
    next_attempt: u32,
    backoff_ms: u64,
    outcome: &'static str,
}

fn retry_signal(
    provider_id: &str,
    key: &str,
    next_attempt: u32,
    backoff: Duration,
    outcome: ProviderOutcome,
) -> RetrySignal {
    RetrySignal {
        provider_id: provider_id.to_string(),
        key_hash: hashed_key(key),
        next_attempt,
        backoff_ms: backoff.as_millis() as u64,
        outcome: outcome.as_str(),
    }
}

fn log_retry_scheduled(
    provider_id: &str,
    key: &str,
    next_attempt: u32,
    backoff: Duration,
    outcome: ProviderOutcome,
) {
    let f = retry_signal(provider_id, key, next_attempt, backoff, outcome);
    info!(
        provider_id = %f.provider_id,
        key_hash = %f.key_hash,
        next_attempt = f.next_attempt,
        backoff_ms = f.backoff_ms,
        outcome = f.outcome,
        "data_provider_fetch_retry"
    );
}

/// The sole runtime implementation of `DataProviderAccess` (AD-003's hybrid
/// resolution model): a handler holds `Arc<dyn DataProviderAccess>` and
/// never a concrete provider or the registry directly.
pub struct RuntimeDataProviderAccess {
    registry: ExternalDataProviderRegistry,
    config: ProviderAccessConfig,
}

impl RuntimeDataProviderAccess {
    /// Wraps `registry` as the handler-facing facade with the default
    /// cross-cutting policy ([`ProviderAccessConfig::default`]).
    pub fn new(registry: ExternalDataProviderRegistry) -> Self {
        Self::with_config(registry, ProviderAccessConfig::default())
    }

    /// Wraps `registry` with an explicit timeout/retry policy (issue #234).
    pub fn with_config(
        registry: ExternalDataProviderRegistry,
        config: ProviderAccessConfig,
    ) -> Self {
        Self { registry, config }
    }
}

#[async_trait]
impl DataProviderAccess for RuntimeDataProviderAccess {
    async fn fetch(
        &self,
        provider_id: &str,
        request: DataRequest,
    ) -> Result<DataResponse, DataProviderError> {
        let started = Instant::now();
        let key = request.key.clone();

        let Some(provider) = self.registry.get(provider_id) else {
            log_fetch(
                provider_id,
                &key,
                started.elapsed(),
                false,
                ProviderOutcome::ProviderMissing,
                1,
            );
            return Err(DataProviderError::ProviderMissing {
                provider_id: provider_id.to_string(),
            });
        };

        let retry = self.config.retry;
        let timeout = self.config.timeout;

        // One span per `fetch` call covering every attempt; carries only the
        // provider id and the hashed key (never the raw key). Spans are not
        // `tracing` events, so this does not change the "exactly one
        // `data_provider_fetch` event per fetch" invariant.
        let span = info_span!("provider_fetch", provider_id, key_hash = %hashed_key(&key));

        async move {
            // `attempt` is the 0-based count of retries already spent, matching
            // `RetryPolicy::allows_retry`/`backoff`'s convention.
            let mut attempt: u32 = 0;
            loop {
                let attempts_so_far = attempt + 1;
                // Each attempt is independently timeout-bounded. Dropping the
                // timed-out future cancels the in-flight provider call.
                let result =
                    match tokio::time::timeout(timeout, provider.fetch(request.clone())).await {
                        Ok(provider_result) => provider_result,
                        Err(_elapsed) => Err(DataProviderError::Timeout),
                    };
                let outcome = ProviderOutcome::from_result(&result);

                if outcome.is_retryable() && retry.allows_retry(attempt) {
                    let next_attempt = attempts_so_far + 1;
                    let backoff = retry.backoff(next_attempt);
                    log_retry_scheduled(provider_id, &key, next_attempt, backoff, outcome);
                    tokio::time::sleep(backoff).await;
                    attempt += 1;
                    continue;
                }

                let cache_hit = result
                    .as_ref()
                    .map(|response| response.cache_hit)
                    .unwrap_or(false);
                log_fetch(
                    provider_id,
                    &key,
                    started.elapsed(),
                    cache_hit,
                    outcome,
                    attempts_so_far,
                );
                return result;
            }
        }
        .instrument(span)
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    use crate::providers::provider::ExternalDataProvider;

    /// Test double returning a canned response for every fetch — shaped
    /// like `testkit`'s future `StaticDataProvider` (Phase 5), used here
    /// only to prove Phase 3's chokepoint behavior.
    struct StaticProvider {
        response: DataResponse,
    }

    #[async_trait]
    impl ExternalDataProvider for StaticProvider {
        async fn fetch(&self, _request: DataRequest) -> Result<DataResponse, DataProviderError> {
            Ok(self.response.clone())
        }
    }

    fn request(key: &str) -> DataRequest {
        DataRequest::new(key, vec![1, 2, 3])
    }

    // -- 3.1: fail-closed provider resolution --------------------------

    #[tokio::test]
    async fn fetch_through_an_unregistered_provider_id_fails_closed() {
        let access = RuntimeDataProviderAccess::new(ExternalDataProviderRegistry::new());

        let err = access
            .fetch("never-registered", request("k"))
            .await
            .unwrap_err();

        assert!(matches!(
            err,
            DataProviderError::ProviderMissing { provider_id } if provider_id == "never-registered"
        ));
    }

    // -- 3.2: fetch signal content (pure — no tracing capture) ----------
    //
    // These assert the observability field *values* on the pure `fetch_signal`
    // / `retry_signal` builders directly, not by capturing through `tracing`
    // dispatch. Capturing was abandoned here for the same reason
    // `crate::effects::observability` abandoned it (CORE-027): `tracing`'s
    // process-wide per-callsite interest cache races under the parallel test
    // sweep and can silently drop a captured event. The `info!` calls in
    // `log_fetch` / `log_retry_scheduled` remain compile-time-checked wiring
    // (the macro fixes field names/values at the callsite).

    #[test]
    fn fetch_signal_hashes_the_key_and_classifies_every_outcome() {
        let latency = Duration::from_millis(7);
        let sig = fetch_signal(
            "pricing",
            "secret-sku-42",
            latency,
            true,
            ProviderOutcome::Success,
            1,
        );

        assert_eq!(sig.provider_id, "pricing");
        assert!(sig.cache_hit);
        assert_eq!(sig.outcome, "success");
        assert_eq!(sig.attempts, 1);
        assert_eq!(sig.latency_ms, 7);
        assert_eq!(
            sig.key_hash,
            hashed_key("secret-sku-42"),
            "the signal correlates on the shared hashed key, reconciled with effects observability"
        );
        assert_ne!(
            sig.key_hash, "secret-sku-42",
            "the raw key must never appear in the signal"
        );

        for (outcome, expected) in [
            (ProviderOutcome::Success, "success"),
            (ProviderOutcome::NotFound, "not_found"),
            (ProviderOutcome::Transient, "transient"),
            (ProviderOutcome::Timeout, "timeout"),
            (ProviderOutcome::Fatal, "fatal"),
            (ProviderOutcome::ProviderMissing, "provider_missing"),
        ] {
            assert_eq!(
                fetch_signal("p", "k", latency, false, outcome, 1).outcome,
                expected
            );
        }
    }

    /// PR1 review G-02 watch-item, now enforced structurally: `Transient`/
    /// `Fatal` carry provider-authored free text that may be sensitive. The
    /// signal builders take only the already-classified [`ProviderOutcome`]
    /// and the key (used solely to hash), so there is *no field* capable of
    /// carrying the message string or the raw key — a leak is impossible by
    /// construction, not merely absent from one code path.
    #[test]
    fn fetch_and_retry_signals_cannot_carry_message_text_or_the_raw_key() {
        let fetch = fetch_signal(
            "p",
            "secret-key",
            Duration::from_millis(1),
            false,
            ProviderOutcome::Fatal,
            2,
        );
        let retry = retry_signal(
            "p",
            "secret-key",
            2,
            Duration::from_millis(3),
            ProviderOutcome::Transient,
        );

        assert_eq!(fetch.key_hash, hashed_key("secret-key"));
        assert_ne!(fetch.key_hash, "secret-key");
        assert_eq!(fetch.outcome, "fatal");
        assert_eq!(fetch.attempts, 2);

        assert_eq!(retry.key_hash, hashed_key("secret-key"));
        assert_ne!(retry.key_hash, "secret-key");
        assert_eq!(retry.outcome, "transient");
        assert_eq!(retry.next_attempt, 2);
        assert_eq!(retry.backoff_ms, 3);
    }

    // -- 3.3: cross-provider isolation -----------------------------------

    /// Two `testkit`-shaped doubles registered under distinct `provider_id`s,
    /// given structurally identical `DataRequest`s, must never cross-resolve
    /// — each fetch returns exactly its own provider's response
    /// (design.md §8).
    #[tokio::test]
    async fn two_providers_under_distinct_ids_never_cross_resolve() {
        let mut registry = ExternalDataProviderRegistry::new();
        registry
            .register(
                "pricing",
                Arc::new(StaticProvider {
                    response: DataResponse {
                        payload: vec![1],
                        cache_hit: false,
                    },
                }),
            )
            .unwrap();
        registry
            .register(
                "jwks",
                Arc::new(StaticProvider {
                    response: DataResponse {
                        payload: vec![2],
                        cache_hit: true,
                    },
                }),
            )
            .unwrap();
        let access = RuntimeDataProviderAccess::new(registry);

        let identical_request = || request("same-key");

        let pricing = access.fetch("pricing", identical_request()).await.unwrap();
        let jwks = access.fetch("jwks", identical_request()).await.unwrap();

        assert_eq!(pricing.payload, vec![1]);
        assert!(!pricing.cache_hit);
        assert_eq!(jwks.payload, vec![2]);
        assert!(jwks.cache_hit);
    }

    // -- #234: timeout + retry ------------------------------------------

    /// A config with `max_attempts` retries and zero backoff — zero backoff
    /// keeps retry tests deterministic (`RetryPolicy::backoff` returns
    /// `Duration::ZERO`, so `sleep` is instant and jitter is irrelevant).
    fn config(timeout: Duration, max_attempts: u32) -> ProviderAccessConfig {
        ProviderAccessConfig {
            timeout,
            retry: RetryPolicy {
                max_attempts,
                base_backoff: Duration::ZERO,
                max_backoff: Duration::ZERO,
            },
        }
    }

    /// A provider whose `fetch` never completes — the timeout must fire.
    /// Counts how many times it is entered so timeout-retry can be asserted.
    struct HangingProvider {
        calls: Arc<Mutex<u32>>,
    }

    #[async_trait]
    impl ExternalDataProvider for HangingProvider {
        async fn fetch(&self, _request: DataRequest) -> Result<DataResponse, DataProviderError> {
            *self.calls.lock().unwrap() += 1;
            std::future::pending().await
        }
    }

    #[tokio::test(start_paused = true)]
    async fn a_fetch_that_exceeds_the_timeout_becomes_a_timeout_error() {
        let mut registry = ExternalDataProviderRegistry::new();
        registry
            .register(
                "slow",
                Arc::new(HangingProvider {
                    calls: Arc::new(Mutex::new(0)),
                }),
            )
            .unwrap();
        // No retries — a single attempt that times out.
        let access =
            RuntimeDataProviderAccess::with_config(registry, config(Duration::from_millis(50), 0));

        let err = access.fetch("slow", request("k")).await.unwrap_err();

        assert_eq!(err, DataProviderError::Timeout);
    }

    #[tokio::test]
    async fn a_transient_failure_is_retried_and_eventually_succeeds() {
        struct FlakyProvider {
            calls: Arc<Mutex<u32>>,
        }
        #[async_trait]
        impl ExternalDataProvider for FlakyProvider {
            async fn fetch(
                &self,
                _request: DataRequest,
            ) -> Result<DataResponse, DataProviderError> {
                let mut calls = self.calls.lock().unwrap();
                *calls += 1;
                if *calls < 3 {
                    Err(DataProviderError::Transient("temporary".to_string()))
                } else {
                    Ok(DataResponse {
                        payload: vec![42],
                        cache_hit: false,
                    })
                }
            }
        }

        let calls = Arc::new(Mutex::new(0));
        let mut registry = ExternalDataProviderRegistry::new();
        registry
            .register(
                "flaky",
                Arc::new(FlakyProvider {
                    calls: calls.clone(),
                }),
            )
            .unwrap();
        let access =
            RuntimeDataProviderAccess::with_config(registry, config(Duration::from_secs(5), 3));

        let response = access.fetch("flaky", request("k")).await.unwrap();

        assert_eq!(response.payload, vec![42]);
        assert_eq!(
            *calls.lock().unwrap(),
            3,
            "two transient failures then success is exactly three attempts"
        );
    }

    #[tokio::test]
    async fn retry_exhaustion_returns_the_final_transient_error() {
        struct AlwaysTransient {
            calls: Arc<Mutex<u32>>,
        }
        #[async_trait]
        impl ExternalDataProvider for AlwaysTransient {
            async fn fetch(
                &self,
                _request: DataRequest,
            ) -> Result<DataResponse, DataProviderError> {
                *self.calls.lock().unwrap() += 1;
                Err(DataProviderError::Transient("still broken".to_string()))
            }
        }

        let calls = Arc::new(Mutex::new(0));
        let mut registry = ExternalDataProviderRegistry::new();
        registry
            .register(
                "flaky",
                Arc::new(AlwaysTransient {
                    calls: calls.clone(),
                }),
            )
            .unwrap();
        // 2 retries => 3 attempts total, all failing.
        let access =
            RuntimeDataProviderAccess::with_config(registry, config(Duration::from_secs(5), 2));

        let err = access.fetch("flaky", request("k")).await.unwrap_err();

        assert!(
            matches!(err, DataProviderError::Transient(msg) if msg == "still broken"),
            "exhausting retries returns the provider's own final error, not a synthetic one"
        );
        assert_eq!(
            *calls.lock().unwrap(),
            3,
            "max_attempts=2 means 3 total attempts before giving up"
        );
    }

    #[tokio::test]
    async fn a_fatal_failure_is_never_retried() {
        struct FatalProvider {
            calls: Arc<Mutex<u32>>,
        }
        #[async_trait]
        impl ExternalDataProvider for FatalProvider {
            async fn fetch(
                &self,
                _request: DataRequest,
            ) -> Result<DataResponse, DataProviderError> {
                *self.calls.lock().unwrap() += 1;
                Err(DataProviderError::Fatal("permanent".to_string()))
            }
        }

        let calls = Arc::new(Mutex::new(0));
        let mut registry = ExternalDataProviderRegistry::new();
        registry
            .register(
                "broken",
                Arc::new(FatalProvider {
                    calls: calls.clone(),
                }),
            )
            .unwrap();
        // Retries are generously allowed, but a fatal error must not use them.
        let access =
            RuntimeDataProviderAccess::with_config(registry, config(Duration::from_secs(5), 5));

        let err = access.fetch("broken", request("k")).await.unwrap_err();

        assert!(matches!(err, DataProviderError::Fatal(_)));
        assert_eq!(
            *calls.lock().unwrap(),
            1,
            "a fatal failure is returned after exactly one attempt — never retried"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_timeout_is_retried_and_exhaustion_returns_a_timeout_error() {
        let calls = Arc::new(Mutex::new(0));
        let mut registry = ExternalDataProviderRegistry::new();
        registry
            .register(
                "slow",
                Arc::new(HangingProvider {
                    calls: calls.clone(),
                }),
            )
            .unwrap();
        // 2 retries => 3 attempts, each timing out.
        let access =
            RuntimeDataProviderAccess::with_config(registry, config(Duration::from_millis(50), 2));

        let err = access.fetch("slow", request("k")).await.unwrap_err();

        assert_eq!(
            err,
            DataProviderError::Timeout,
            "a timeout is explicitly retryable, and exhausting retries still yields Timeout"
        );
        assert_eq!(
            *calls.lock().unwrap(),
            3,
            "each of the 3 attempts entered the provider before timing out"
        );
    }
}
