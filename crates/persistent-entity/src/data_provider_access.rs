//! Handler-facing external data access port (CORE-019A Phase 1, AD-001/
//! AD-004/AD-009).
//!
//! Mirrors [`crate::effect_acceptor`]: the trait + DTOs live here so a
//! handler depends only on this crate, never on `ego-runtime`'s
//! `RuntimeDataProviderAccess` (AD-3's dependency direction, reused
//! verbatim). `DataRequest`/`DataResponse` are deliberately opaque bytes —
//! the runtime stays transport-unaware; the handler serializes/deserializes.

use async_trait::async_trait;
use thiserror::Error;

/// Opaque request handed to a registered external data provider (AD-004).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataRequest {
    /// The provider-defined lookup key (e.g. a JWKS `kid`, a pricing SKU).
    pub key: String,
    /// Opaque request payload the provider interprets; never inspected by
    /// the runtime chokepoint.
    pub payload: Vec<u8>,
}

/// Opaque response returned by a registered external data provider.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataResponse {
    /// Opaque response payload; never inspected by the runtime chokepoint.
    pub payload: Vec<u8>,
    /// Whether the provider served this from its own cache (AD-006) rather
    /// than a fresh remote fetch. The runtime never inspects cache
    /// contents, only this flag.
    pub cache_hit: bool,
}

/// Read-side error classification (AD-007). Every variant traces to an
/// existing in-repo error: `Transient`/`Fatal` mirror the read-side
/// classification convention, `NotFound` traces to
/// `KeyResolverError::KeyNotFound`, `ProviderMissing` traces to CORE-019's
/// `ExecutorMissing`.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum DataProviderError {
    /// A retryable failure; no retry/backoff policy exists in this slice
    /// (§9 non-goals) — the handler decides whether/how to retry.
    #[error("transient data provider failure: {0}")]
    Transient(String),
    /// A non-retryable failure.
    #[error("fatal data provider failure: {0}")]
    Fatal(String),
    /// The provider was resolved but has no data for `key`.
    #[error("no data found for key '{key}'")]
    NotFound {
        /// The key that had no matching data.
        key: String,
    },
    /// No provider is registered for `provider_id` — the fail-closed
    /// resolution case (spec: "Fail-Closed Provider Resolution").
    #[error("no provider registered for provider_id '{provider_id}'")]
    ProviderMissing {
        /// The `provider_id` that had no registered owner.
        provider_id: String,
    },
}

/// Handler-facing external data access port (AD-003 hybrid model). A
/// handler holds `Arc<dyn DataProviderAccess>` and never a concrete
/// provider or the registry directly. The sole runtime implementation
/// (`RuntimeDataProviderAccess`, `ego-runtime`) performs the registry
/// lookup and is the single observability chokepoint (Phase 3).
#[async_trait]
pub trait DataProviderAccess: Send + Sync {
    /// Fetches data from the provider registered under `provider_id`.
    ///
    /// # Errors
    ///
    /// Returns [`DataProviderError::ProviderMissing`] when no provider is
    /// registered for `provider_id`, or the provider's own classified
    /// failure otherwise.
    async fn fetch(
        &self,
        provider_id: &str,
        request: DataRequest,
    ) -> Result<DataResponse, DataProviderError>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use std::sync::Arc;

    /// Echoes the request payload back — proves the trait object is
    /// callable and that the response actually carries data derived from
    /// the request (not a hardcoded constant).
    struct AlwaysEcho;

    #[async_trait]
    impl DataProviderAccess for AlwaysEcho {
        async fn fetch(
            &self,
            _provider_id: &str,
            request: DataRequest,
        ) -> Result<DataResponse, DataProviderError> {
            Ok(DataResponse {
                payload: request.payload,
                cache_hit: false,
            })
        }
    }

    #[test]
    fn data_provider_access_is_object_safe() {
        let access: Arc<dyn DataProviderAccess> = Arc::new(AlwaysEcho);

        let response = futures::executor::block_on(access.fetch(
            "prov-a",
            DataRequest {
                key: "k".to_string(),
                payload: vec![1, 2, 3],
            },
        ))
        .unwrap();

        assert_eq!(response.payload, vec![1, 2, 3]);
        assert!(!response.cache_hit);
    }

    /// Triangulation: a second implementation that returns a typed error for
    /// one key and a real (non-hardcoded) response for another, proving the
    /// `DataProviderError` variants round-trip through the trait object and
    /// the response payload is not a fixed constant.
    struct KeyGatedProvider;

    #[async_trait]
    impl DataProviderAccess for KeyGatedProvider {
        async fn fetch(
            &self,
            _provider_id: &str,
            request: DataRequest,
        ) -> Result<DataResponse, DataProviderError> {
            if request.key == "missing" {
                return Err(DataProviderError::NotFound { key: request.key });
            }
            Ok(DataResponse {
                payload: request.payload,
                cache_hit: true,
            })
        }
    }

    #[test]
    fn data_provider_access_propagates_typed_errors_through_the_trait_object() {
        let access: Arc<dyn DataProviderAccess> = Arc::new(KeyGatedProvider);

        let err = futures::executor::block_on(access.fetch(
            "prov-a",
            DataRequest {
                key: "missing".to_string(),
                payload: vec![],
            },
        ))
        .unwrap_err();
        assert_eq!(
            err,
            DataProviderError::NotFound {
                key: "missing".to_string()
            }
        );

        let ok = futures::executor::block_on(access.fetch(
            "prov-a",
            DataRequest {
                key: "present".to_string(),
                payload: vec![9, 8, 7],
            },
        ))
        .unwrap();
        assert_eq!(ok.payload, vec![9, 8, 7]);
        assert!(ok.cache_hit);
    }
}
