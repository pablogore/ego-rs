//! External data provider SPI (CORE-019A Phase 2, AD-002/AD-004).
//!
//! Apps implement this trait; the runtime never does. Cache-first contract
//! (AD-006/AD-013): `fetch` MUST resolve from local state so the
//! `futures_executor::block_on` sync bridge stays correct — remote warm-up
//! happens outside `fetch`. Deliberate superset of `security_jwt::KeyResolver`
//! (AD-011): a future retrofit is a thin adapter, not a rewrite.

use async_trait::async_trait;
use persistent_entity::data_provider_access::{DataProviderError, DataRequest, DataResponse};

/// One provider = one kind of external data, registered under a
/// `provider_id` (see [`super::registry::ExternalDataProviderRegistry`]).
/// Object-safe — stored behind `Arc<dyn ExternalDataProvider>`.
#[async_trait]
pub trait ExternalDataProvider: Send + Sync {
    /// Fetches data for `request`. MUST be cache-first (AD-006): remote
    /// warm-up happens outside this call, never inside it.
    async fn fetch(&self, request: DataRequest) -> Result<DataResponse, DataProviderError>;

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

    /// Cache-first provider shaped like `testkit`'s future `StaticDataProvider`
    /// (Phase 5) — returns a canned response with no I/O, proving the
    /// `block_on` sync bridge stays correct (mirrors `key_resolver.rs`'s
    /// `local_key_resolver_is_runtime_free`).
    struct StaticDataProvider {
        response: DataResponse,
    }

    #[async_trait]
    impl ExternalDataProvider for StaticDataProvider {
        async fn fetch(&self, _request: DataRequest) -> Result<DataResponse, DataProviderError> {
            Ok(self.response.clone())
        }
    }

    #[test]
    fn external_data_provider_is_object_safe_and_cache_first_via_block_on() {
        let provider: Arc<dyn ExternalDataProvider> = Arc::new(StaticDataProvider {
            response: DataResponse {
                payload: vec![9, 9],
                cache_hit: true,
            },
        });

        let response = futures_executor::block_on(provider.fetch(DataRequest {
            key: "k".to_string(),
            payload: vec![],
        }))
        .unwrap();

        assert_eq!(response.payload, vec![9, 9]);
        assert!(response.cache_hit);
    }

    /// Triangulation: a distinct provider instance carrying a different
    /// canned response, resolved through the same trait object type,
    /// proving the response is not a fixed constant baked into the trait.
    #[test]
    fn a_second_provider_instance_returns_its_own_distinct_response() {
        let provider: Arc<dyn ExternalDataProvider> = Arc::new(StaticDataProvider {
            response: DataResponse {
                payload: vec![1],
                cache_hit: false,
            },
        });

        let response = futures_executor::block_on(provider.fetch(DataRequest {
            key: "other".to_string(),
            payload: vec![],
        }))
        .unwrap();

        assert_eq!(response.payload, vec![1]);
        assert!(!response.cache_hit);
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
