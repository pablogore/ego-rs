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

/// One provider = one kind of external data, registered under a
/// `provider_id` (see [`super::registry::ExternalDataProviderRegistry`]).
/// Object-safe — stored behind `Arc<dyn ExternalDataProvider>`.
#[async_trait]
pub trait ExternalDataProvider: Send + Sync {
    /// Fetches data for `request`. May perform real I/O — there is no
    /// cache-first precondition (AD-006).
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

        let response = provider
            .fetch(DataRequest {
                key: "k".to_string(),
                payload: vec![],
            })
            .await
            .unwrap();

        assert_eq!(response.payload, vec![9, 9]);
        assert!(response.cache_hit);
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
            .fetch(DataRequest {
                key: "other".to_string(),
                payload: vec![],
            })
            .await
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
