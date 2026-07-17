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
//! actual chokepoint rather than inlining `tracing::info!` there).

use std::time::{Duration, Instant};

use async_trait::async_trait;
use persistent_entity::data_provider_access::{
    DataProviderAccess, DataProviderError, DataRequest, DataResponse,
};
use sha2::{Digest, Sha256};
use tracing::info;

use super::registry::ExternalDataProviderRegistry;

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
            Self::Fatal => "fatal",
            Self::ProviderMissing => "provider_missing",
        }
    }

    fn from_result(result: &Result<DataResponse, DataProviderError>) -> Self {
        match result {
            Ok(_) => Self::Success,
            Err(DataProviderError::NotFound { .. }) => Self::NotFound,
            Err(DataProviderError::Transient(_)) => Self::Transient,
            Err(DataProviderError::Fatal(_)) => Self::Fatal,
            Err(DataProviderError::ProviderMissing { .. }) => Self::ProviderMissing,
        }
    }
}

/// Redacts `key` to a short, deterministic, non-reversible hash — never the
/// raw key — for cross-log correlation. Mirrors
/// `crate::effects::observability::hashed_key` verbatim (deterministic
/// SHA-256 prefix, not `DefaultHasher`, so the same key correlates across
/// process restarts).
fn hashed_key(key: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(key.as_bytes());
    let digest = hasher.finalize();
    digest[..4].iter().map(|b| format!("{b:02x}")).collect()
}

/// Emits the one observability event a fetch attempt produces (AD-008,
/// spec: "Fetch Observability Signals"). Never receives `payload`; never
/// receives a `DataProviderError::Transient`/`Fatal` message string — only
/// the already-classified [`ProviderOutcome`].
fn log_fetch(provider_id: &str, key: &str, latency: Duration, cache_hit: bool, outcome: ProviderOutcome) {
    info!(
        provider_id,
        key_hash = %hashed_key(key),
        latency_ms = latency.as_millis() as u64,
        cache_hit,
        outcome = outcome.as_str(),
        "data_provider_fetch"
    );
}

/// The sole runtime implementation of `DataProviderAccess` (AD-003's hybrid
/// resolution model): a handler holds `Arc<dyn DataProviderAccess>` and
/// never a concrete provider or the registry directly.
pub struct RuntimeDataProviderAccess {
    registry: ExternalDataProviderRegistry,
}

impl RuntimeDataProviderAccess {
    /// Wraps `registry` as the handler-facing facade.
    pub fn new(registry: ExternalDataProviderRegistry) -> Self {
        Self { registry }
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
            );
            return Err(DataProviderError::ProviderMissing {
                provider_id: provider_id.to_string(),
            });
        };

        let result = provider.fetch(request).await;
        let cache_hit = result.as_ref().map(|response| response.cache_hit).unwrap_or(false);
        log_fetch(
            provider_id,
            &key,
            started.elapsed(),
            cache_hit,
            ProviderOutcome::from_result(&result),
        );
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
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
        DataRequest {
            key: key.to_string(),
            payload: vec![1, 2, 3],
        }
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

    // -- 3.2: tracing event content --------------------------------------

    #[derive(Default, Clone, Debug)]
    struct CapturedEvent {
        fields: HashMap<String, String>,
    }

    struct FieldRecorder<'a>(&'a mut HashMap<String, String>);

    impl tracing::field::Visit for FieldRecorder<'_> {
        fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
            self.0.insert(field.name().to_string(), format!("{value:?}"));
        }
        fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
            self.0.insert(field.name().to_string(), value.to_string());
        }
        fn record_u64(&mut self, field: &tracing::field::Field, value: u64) {
            self.0.insert(field.name().to_string(), value.to_string());
        }
        fn record_bool(&mut self, field: &tracing::field::Field, value: bool) {
            self.0.insert(field.name().to_string(), value.to_string());
        }
    }

    struct TestSubscriber {
        events: Arc<Mutex<Vec<CapturedEvent>>>,
    }

    impl tracing::Subscriber for TestSubscriber {
        fn enabled(&self, _metadata: &tracing::Metadata<'_>) -> bool {
            true
        }
        fn new_span(&self, _span: &tracing::span::Attributes<'_>) -> tracing::span::Id {
            tracing::span::Id::from_u64(1)
        }
        fn record(&self, _span: &tracing::span::Id, _values: &tracing::span::Record<'_>) {}
        fn record_follows_from(&self, _span: &tracing::span::Id, _follows: &tracing::span::Id) {}
        fn event(&self, event: &tracing::Event<'_>) {
            let mut fields = HashMap::new();
            event.record(&mut FieldRecorder(&mut fields));
            self.events.lock().unwrap().push(CapturedEvent { fields });
        }
        fn enter(&self, _span: &tracing::span::Id) {}
        fn exit(&self, _span: &tracing::span::Id) {}
    }

    /// Awaits `f` under a subscriber that records every emitted event's
    /// fields — adapts `crate::effects::observability`'s sync test-double
    /// pattern to this chokepoint's `async fn fetch`. Relies on `#[tokio::
    /// test]`'s default current-thread flavor (single OS thread, no task
    /// migration across the `.await` below), so the thread-local default
    /// subscriber set by `set_default` reliably covers the whole future.
    ///
    /// CORE-027 flaky-triage fix: previously extracted the recorded events
    /// via `Arc::try_unwrap(events).unwrap()`, asserting exclusive ownership
    /// of the `Arc` at that point. Under a full-crate parallel sweep
    /// (`--test-threads=64`, dozens of `tracing` dispatchers being installed
    /// and torn down across threads concurrently), that `try_unwrap`
    /// intermittently panicked — `tracing_core`'s global per-callsite
    /// interest cache transiently holds an extra `Dispatch` clone while
    /// rebuilding under contention, which briefly bumps this subscriber's
    /// `Arc` strong count above 1 even though `guard` has already restored
    /// the prior default and no further events can route into `self.0`.
    /// Exclusive ownership was never actually required — only the recorded
    /// data is — so this reads the `Vec` out through the `Mutex` instead,
    /// which is correct regardless of how many (harmless) extra clones of
    /// the `Arc` transiently exist.
    async fn capture_events<Fut>(f: Fut) -> Vec<CapturedEvent>
    where
        Fut: std::future::Future<Output = ()>,
    {
        let events = Arc::new(Mutex::new(Vec::new()));
        let subscriber = TestSubscriber {
            events: events.clone(),
        };
        let guard = tracing::subscriber::set_default(subscriber);
        f.await;
        drop(guard);
        let recorded = events.lock().unwrap().clone();
        recorded
    }

    fn find_message(events: &[CapturedEvent], message: &str) -> CapturedEvent {
        events
            .iter()
            .find(|e| e.fields.get("message").map(|m| m.as_str()) == Some(message))
            .unwrap_or_else(|| panic!("no captured event with message {message:?}"))
            .clone()
    }

    #[tokio::test]
    async fn successful_fetch_emits_one_event_with_hashed_key_latency_cache_hit_and_outcome() {
        let mut registry = ExternalDataProviderRegistry::new();
        registry
            .register(
                "pricing",
                Arc::new(StaticProvider {
                    response: DataResponse {
                        payload: vec![9, 9],
                        cache_hit: true,
                    },
                }),
            )
            .unwrap();
        let access = RuntimeDataProviderAccess::new(registry);

        let events = capture_events(async {
            access.fetch("pricing", request("secret-sku-42")).await.unwrap();
        })
        .await;

        assert_eq!(events.len(), 1, "exactly one event per fetch attempt");
        let event = find_message(&events, "data_provider_fetch");

        assert_eq!(event.fields.get("provider_id").map(String::as_str), Some("pricing"));
        assert_eq!(event.fields.get("cache_hit").map(String::as_str), Some("true"));
        assert_eq!(event.fields.get("outcome").map(String::as_str), Some("success"));
        assert!(event.fields.contains_key("latency_ms"));

        let key_hash = event.fields.get("key_hash").expect("key_hash field present");
        assert_ne!(key_hash, "secret-sku-42", "the raw key must never appear in the signal");
    }

    #[tokio::test]
    async fn provider_missing_fetch_emits_an_event_with_provider_missing_outcome() {
        let access = RuntimeDataProviderAccess::new(ExternalDataProviderRegistry::new());

        let events = capture_events(async {
            let _ = access.fetch("never-registered", request("k")).await;
        })
        .await;

        let event = find_message(&events, "data_provider_fetch");
        assert_eq!(
            event.fields.get("outcome").map(String::as_str),
            Some("provider_missing")
        );
        assert_eq!(event.fields.get("cache_hit").map(String::as_str), Some("false"));
    }

    /// PR1 review G-02 watch-item: `Transient`/`Fatal` carry provider-authored
    /// free text that may be sensitive. The emitted signal must carry only
    /// the `ProviderOutcome` classification, never that message string.
    #[tokio::test]
    async fn transient_and_fatal_errors_never_leak_their_message_text_into_the_signal() {
        struct FailingProvider;

        const DISTINCTIVE_TRANSIENT_MSG: &str = "TRANSIENT-MESSAGE-MUST-NEVER-LEAK-7c1a";
        const DISTINCTIVE_FATAL_MSG: &str = "FATAL-MESSAGE-MUST-NEVER-LEAK-9d2b";

        #[async_trait]
        impl ExternalDataProvider for FailingProvider {
            async fn fetch(&self, request: DataRequest) -> Result<DataResponse, DataProviderError> {
                if request.key == "transient" {
                    Err(DataProviderError::Transient(DISTINCTIVE_TRANSIENT_MSG.to_string()))
                } else {
                    Err(DataProviderError::Fatal(DISTINCTIVE_FATAL_MSG.to_string()))
                }
            }
        }

        let mut registry = ExternalDataProviderRegistry::new();
        registry.register("flaky", Arc::new(FailingProvider)).unwrap();
        let access = RuntimeDataProviderAccess::new(registry);

        let events = capture_events(async {
            let _ = access.fetch("flaky", request("transient")).await;
            let _ = access.fetch("flaky", request("fatal")).await;
        })
        .await;

        assert_eq!(events.len(), 2);
        for event in &events {
            for value in event.fields.values() {
                assert!(
                    !value.contains(DISTINCTIVE_TRANSIENT_MSG),
                    "transient message text leaked: {value:?}"
                );
                assert!(
                    !value.contains(DISTINCTIVE_FATAL_MSG),
                    "fatal message text leaked: {value:?}"
                );
            }
        }
        assert_eq!(
            find_message(&[events[0].clone()], "data_provider_fetch")
                .fields
                .get("outcome")
                .map(String::as_str),
            Some("transient")
        );
        assert_eq!(
            events[1].fields.get("outcome").map(String::as_str),
            Some("fatal")
        );
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
}
