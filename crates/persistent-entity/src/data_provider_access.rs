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
///
/// `Debug` is hand-written, never derived: `key` and `payload` are exactly
/// the fields design.md forbids from ever appearing in a log or span
/// (§7: "payload never logged"). Deriving `Debug` here would let a future
/// `tracing::debug!(?request)` in the Phase 3 chokepoint leak them by
/// accident; the real observability hash of `key` is computed by the
/// chokepoint directly from this field, not through `Debug`.
#[derive(Clone, PartialEq, Eq)]
pub struct DataRequest {
    /// The provider-defined lookup key (e.g. a JWKS `kid`, a pricing SKU).
    pub key: String,
    /// Opaque request payload the provider interprets; never inspected by
    /// the runtime chokepoint.
    pub payload: Vec<u8>,
}

impl std::fmt::Debug for DataRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DataRequest")
            .field("key", &format_args!("<redacted, {} bytes>", self.key.len()))
            .field(
                "payload",
                &format_args!("<redacted, {} bytes>", self.payload.len()),
            )
            .finish()
    }
}

/// Opaque response returned by a registered external data provider.
///
/// `Debug` is hand-written for the same reason as [`DataRequest`]: `payload`
/// must never appear in a log or span.
#[derive(Clone, PartialEq, Eq)]
pub struct DataResponse {
    /// Opaque response payload; never inspected by the runtime chokepoint.
    pub payload: Vec<u8>,
    /// Whether the provider served this from its own cache (AD-006) rather
    /// than a fresh remote fetch. The runtime never inspects cache
    /// contents, only this flag.
    pub cache_hit: bool,
}

impl std::fmt::Debug for DataResponse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DataResponse")
            .field(
                "payload",
                &format_args!("<redacted, {} bytes>", self.payload.len()),
            )
            .field("cache_hit", &self.cache_hit)
            .finish()
    }
}

/// Read-side error classification (AD-007). Every variant traces to an
/// existing in-repo error: `Transient`/`Fatal` mirror the read-side
/// classification convention, `NotFound` traces to
/// `KeyResolverError::KeyNotFound`, `ProviderMissing` traces to CORE-019's
/// `ExecutorMissing`.
///
/// `Debug` is hand-written, never derived: `NotFound`'s `key` is the same
/// log-sensitive field as [`DataRequest::key`] and must never appear raw in
/// a log or span, in `Display` or `Debug`.
#[derive(Clone, PartialEq, Eq, Error)]
pub enum DataProviderError {
    /// A retryable failure; no retry/backoff policy exists in this slice
    /// (§9 non-goals) — the handler decides whether/how to retry.
    #[error("transient data provider failure: {0}")]
    Transient(String),
    /// A non-retryable failure.
    #[error("fatal data provider failure: {0}")]
    Fatal(String),
    /// The provider was resolved but has no data for `key`.
    #[error("no data found for the requested key")]
    NotFound {
        /// The key that had no matching data. Never rendered raw — see the
        /// hand-written `Debug` impl below.
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

impl std::fmt::Debug for DataProviderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Transient(msg) => f.debug_tuple("Transient").field(msg).finish(),
            Self::Fatal(msg) => f.debug_tuple("Fatal").field(msg).finish(),
            Self::NotFound { key } => f
                .debug_struct("NotFound")
                .field("key", &format_args!("<redacted, {} bytes>", key.len()))
                .finish(),
            Self::ProviderMissing { provider_id } => f
                .debug_struct("ProviderMissing")
                .field("provider_id", provider_id)
                .finish(),
        }
    }
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

    #[test]
    fn debug_output_never_contains_raw_key_or_payload() {
        let request = DataRequest {
            key: "secret-kid-123".to_string(),
            payload: b"super-sensitive-bytes".to_vec(),
        };
        let response = DataResponse {
            payload: b"super-sensitive-bytes".to_vec(),
            cache_hit: true,
        };
        let error = DataProviderError::NotFound {
            key: "secret-kid-123".to_string(),
        };

        let request_debug = format!("{request:?}");
        let response_debug = format!("{response:?}");
        let error_debug = format!("{error:?}");
        let error_display = format!("{error}");

        for rendered in [&request_debug, &response_debug, &error_debug, &error_display] {
            assert!(
                !rendered.contains("secret-kid-123"),
                "raw key leaked in: {rendered}"
            );
            assert!(
                !rendered.contains("super-sensitive-bytes"),
                "raw payload leaked in: {rendered}"
            );
        }
    }
}
