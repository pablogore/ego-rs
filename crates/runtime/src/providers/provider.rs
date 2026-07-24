//! External data provider SPI (CORE-019A Phase 2, AD-002/AD-004).
//!
//! Apps implement this trait; the runtime never does. `fetch` may do real
//! I/O — `handle_command`/`apply_event`/`apply_events` are already `async
//! fn`, so there is no synchronous call site requiring a `block_on` bridge
//! (AD-006 correction, PR1 review: an earlier cache-first mandate, copied
//! from `KeyResolver`'s justified-but-specific sync call site, did not
//! apply here and was dropped). Caching, if a provider wants it, is the
//! provider's own concern. Deliberate superset of `security_jwt::KeyResolver`
//! (AD-011): a future retrofit is a thin adapter, not a rewrite.

use async_trait::async_trait;
use persistent_entity::data_provider_access::{DataProviderError, DataRequest, DataResponse};

/// A provider's self-reported readiness (issue #234).
///
/// This is **provider-level** health — whether *this* provider can currently
/// serve fetches — and is deliberately kept distinct from *process* liveness:
/// a provider reporting [`ProviderHealth::Unhealthy`] means its backing
/// dependency is degraded or unavailable, never that the process is dead or
/// should terminate. There is intentionally no free-text reason field: like a
/// [`DataProviderError::Fatal`] message, an arbitrary provider-authored string
/// could leak sensitive detail into a readiness signal, so health is a closed,
/// non-leaking classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderHealth {
    /// Ready to serve fetches.
    Healthy,
    /// Registered but currently unable to serve reliably (its backing
    /// dependency is degraded or unavailable). Not a statement about process
    /// liveness.
    Unhealthy,
}

/// One provider = one kind of external data, registered under a
/// `provider_id` (see [`super::registry::ExternalDataProviderRegistry`]).
/// Object-safe — stored behind `Arc<dyn ExternalDataProvider>`.
#[async_trait]
pub trait ExternalDataProvider: Send + Sync {
    /// Fetches data for `request`. May perform real I/O — there is no
    /// cache-first precondition (AD-006).
    async fn fetch(&self, request: DataRequest) -> Result<DataResponse, DataProviderError>;

    /// Reports whether this provider is ready to serve (issue #234). Default
    /// [`ProviderHealth::Healthy`], mirroring [`Self::shutdown`]'s default
    /// no-op: a provider with no active health check to run remains valid and
    /// is treated as ready — implementing this method is purely opt-in, so
    /// existing providers keep compiling and behaving unchanged. A provider
    /// that *does* have a cheap liveness signal (a warm connection pool, a
    /// cached JWKS document, ...) overrides this; it MUST NOT perform an
    /// expensive fetch just to answer, and MUST NOT block indefinitely.
    async fn health(&self) -> ProviderHealth {
        ProviderHealth::Healthy
    }

    /// Tears down any long-lived resource (HTTP pool, gRPC channel, Redis/S3
    /// client, ...). Default no-op; driven by `register_async_teardown`
    /// (a later work unit) for providers that hold such a resource.
    async fn shutdown(&self) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use persistent_entity::data_provider_access::{DataProviderError, DataRequest, DataResponse};
    use std::sync::Arc;

    /// Test double shaped like `testkit`'s future `StaticDataProvider`
    /// (Phase 5) — returns a canned response, proving the trait object is
    /// callable through a real `async` executor (no `block_on` bridge: the
    /// real call site, `handle_command`, is already async — AD-006
    /// correction).
    struct StaticDataProvider {
        response: DataResponse,
    }

    #[async_trait]
    impl ExternalDataProvider for StaticDataProvider {
        async fn fetch(&self, _request: DataRequest) -> Result<DataResponse, DataProviderError> {
            Ok(self.response.clone())
        }
    }

    #[tokio::test]
    async fn external_data_provider_is_object_safe() {
        let provider: Arc<dyn ExternalDataProvider> = Arc::new(StaticDataProvider {
            response: DataResponse {
                payload: vec![9, 9],
                cache_hit: true,
            },
        });

        let response = provider.fetch(DataRequest::new("k", vec![])).await.unwrap();

        assert_eq!(response.payload, vec![9, 9]);
        assert!(response.cache_hit);
        // The default `health()` is reachable through the trait object and
        // reports `Healthy` — proving the new method keeps the trait
        // object-safe (issue #234).
        assert_eq!(provider.health().await, ProviderHealth::Healthy);
    }

    /// Triangulation: a distinct provider instance carrying a different
    /// canned response, resolved through the same trait object type,
    /// proving the response is not a fixed constant baked into the trait.
    #[tokio::test]
    async fn a_second_provider_instance_returns_its_own_distinct_response() {
        let provider: Arc<dyn ExternalDataProvider> = Arc::new(StaticDataProvider {
            response: DataResponse {
                payload: vec![1],
                cache_hit: false,
            },
        });

        let response = provider
            .fetch(DataRequest::new("other", vec![]))
            .await
            .unwrap();

        assert_eq!(response.payload, vec![1]);
        assert!(!response.cache_hit);
    }

    /// A provider that overrides `health()` to report `Unhealthy` — proves the
    /// default is genuinely overridable and that an unhealthy provider is a
    /// valid, still-usable trait object (its `fetch` is unaffected). Issue #234.
    #[tokio::test]
    async fn a_provider_may_override_health_to_report_unhealthy() {
        struct UnhealthyProvider;

        #[async_trait]
        impl ExternalDataProvider for UnhealthyProvider {
            async fn fetch(
                &self,
                _request: DataRequest,
            ) -> Result<DataResponse, DataProviderError> {
                Ok(DataResponse {
                    payload: vec![],
                    cache_hit: false,
                })
            }
            async fn health(&self) -> ProviderHealth {
                ProviderHealth::Unhealthy
            }
        }

        let provider: Arc<dyn ExternalDataProvider> = Arc::new(UnhealthyProvider);
        assert_eq!(provider.health().await, ProviderHealth::Unhealthy);
    }

    /// A provider that implements only `fetch` (no `health` override) reports
    /// `Healthy` — the opt-in default that keeps every pre-#234 provider valid.
    #[tokio::test]
    async fn a_provider_without_a_health_override_defaults_to_healthy() {
        struct FetchOnlyProvider;

        #[async_trait]
        impl ExternalDataProvider for FetchOnlyProvider {
            async fn fetch(
                &self,
                _request: DataRequest,
            ) -> Result<DataResponse, DataProviderError> {
                Ok(DataResponse {
                    payload: vec![],
                    cache_hit: false,
                })
            }
        }

        let provider: Arc<dyn ExternalDataProvider> = Arc::new(FetchOnlyProvider);
        assert_eq!(provider.health().await, ProviderHealth::Healthy);
    }

    /// Proves `shutdown`'s default no-op exists and runs without requiring
    /// an override — the minimal case for long-lived-resource providers
    /// that don't need teardown (AD-006).
    #[tokio::test]
    async fn shutdown_default_is_a_no_op() {
        struct NoShutdownOverride;

        #[async_trait]
        impl ExternalDataProvider for NoShutdownOverride {
            async fn fetch(&self, request: DataRequest) -> Result<DataResponse, DataProviderError> {
                Ok(DataResponse {
                    payload: request.payload,
                    cache_hit: false,
                })
            }
        }

        NoShutdownOverride.shutdown().await;
    }
}
