//! Handler-facing external data access port (CORE-019A Phase 1, AD-001/
//! AD-004/AD-009).
//!
//! Mirrors [`crate::effect_acceptor`]: the trait + DTOs live here so a
//! handler depends only on this crate, never on `ego-runtime`'s
//! `RuntimeDataProviderAccess` (AD-3's dependency direction, reused
//! verbatim). `DataRequest`/`DataResponse` are deliberately opaque bytes —
//! the runtime stays transport-unaware; the handler serializes/deserializes.

use async_trait::async_trait;
use ego_domain::TenantId;
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
    /// The authoritative tenant this fetch is scoped to (issue #234). This is
    /// the runtime's already-validated [`TenantId`] (`ego_domain`), never an
    /// untrusted free-text hint and never a second tenant representation — the
    /// caller must pass the same authoritative identity the rest of the
    /// command context already carries. `None` is the single-tenant /
    /// tenant-agnostic case (a provider that serves one tenant, or data that
    /// is not tenant-scoped), which keeps the pre-#234 usage working
    /// unchanged. There is no ambient/global tenant state: a provider that
    /// needs tenant scoping reads it here, from the request it was handed.
    pub tenant: Option<TenantId>,
}

impl DataRequest {
    /// A tenant-agnostic request (the single-tenant / not-tenant-scoped case).
    /// Attach an authoritative tenant with [`DataRequest::with_tenant`] or
    /// construct one directly with [`DataRequest::for_tenant`].
    pub fn new(key: impl Into<String>, payload: Vec<u8>) -> Self {
        Self {
            key: key.into(),
            payload,
            tenant: None,
        }
    }

    /// A request scoped to an authoritative `tenant`.
    pub fn for_tenant(key: impl Into<String>, payload: Vec<u8>, tenant: TenantId) -> Self {
        Self {
            key: key.into(),
            payload,
            tenant: Some(tenant),
        }
    }

    /// Scopes this request to `tenant`, returning the updated request
    /// (builder-style). Overwrites any tenant already set.
    pub fn with_tenant(mut self, tenant: TenantId) -> Self {
        self.tenant = Some(tenant);
        self
    }
}

impl std::fmt::Debug for DataRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DataRequest")
            .field("key", &format_args!("<redacted, {} bytes>", self.key.len()))
            .field(
                "payload",
                &format_args!("<redacted, {} bytes>", self.payload.len()),
            )
            // `tenant` is authoritative identity, not a secret — the runtime's
            // existing effect observability logs the tenant id verbatim
            // (`crate::effects::observability`), so it is safe to render here,
            // unlike `key`/`payload`.
            .field("tenant", &self.tenant.as_ref().map(TenantId::as_str))
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
    /// A retryable failure. The uniform retry orchestration at the runtime
    /// access chokepoint (`ego_runtime::providers::access`, issue #234) retries
    /// this class; the free-text message is provider-authored and, like
    /// [`Self::Fatal`], is never logged.
    #[error("transient data provider failure: {0}")]
    Transient(String),
    /// A non-retryable failure.
    #[error("fatal data provider failure: {0}")]
    Fatal(String),
    /// The fetch did not complete within the access chokepoint's configured
    /// timeout (issue #234). Synthesized centrally by
    /// `ego_runtime::providers::access` — an individual provider never has to
    /// enforce or report its own timeout. Classified as retryable by the
    /// chokepoint's retry policy, distinct from [`Self::Transient`] so a
    /// timeout is queryable/alertable on its own. Carries no free text (there
    /// is nothing provider-authored to carry) and no elapsed duration (a
    /// high-cardinality value that belongs in the span, not the error).
    #[error("data provider request timed out")]
    Timeout,
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
            Self::Timeout => f.write_str("Timeout"),
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

    #[tokio::test]
    async fn data_provider_access_is_object_safe() {
        let access: Arc<dyn DataProviderAccess> = Arc::new(AlwaysEcho);

        let response = access
            .fetch("prov-a", DataRequest::new("k", vec![1, 2, 3]))
            .await
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

    #[tokio::test]
    async fn data_provider_access_propagates_typed_errors_through_the_trait_object() {
        let access: Arc<dyn DataProviderAccess> = Arc::new(KeyGatedProvider);

        let err = access
            .fetch("prov-a", DataRequest::new("missing", vec![]))
            .await
            .unwrap_err();
        assert_eq!(
            err,
            DataProviderError::NotFound {
                key: "missing".to_string()
            }
        );

        let ok = access
            .fetch("prov-a", DataRequest::new("present", vec![9, 8, 7]))
            .await
            .unwrap();
        assert_eq!(ok.payload, vec![9, 8, 7]);
        assert!(ok.cache_hit);
    }

    /// A recording double that captures the exact `tenant` each fetch was
    /// handed — the proof surface for "authoritative tenant reaches provider".
    struct TenantRecordingProvider {
        seen: std::sync::Mutex<Vec<Option<TenantId>>>,
    }

    #[async_trait]
    impl DataProviderAccess for TenantRecordingProvider {
        async fn fetch(
            &self,
            _provider_id: &str,
            request: DataRequest,
        ) -> Result<DataResponse, DataProviderError> {
            self.seen.lock().unwrap().push(request.tenant.clone());
            Ok(DataResponse {
                payload: request.payload,
                cache_hit: false,
            })
        }
    }

    #[tokio::test]
    async fn authoritative_tenant_reaches_the_provider_through_the_request() {
        let provider = Arc::new(TenantRecordingProvider {
            seen: std::sync::Mutex::new(Vec::new()),
        });
        let access: Arc<dyn DataProviderAccess> = provider.clone();

        let tenant = TenantId::new("tenant-a").unwrap();
        access
            .fetch(
                "prov-a",
                DataRequest::for_tenant("k", vec![], tenant.clone()),
            )
            .await
            .unwrap();
        // Single-tenant / tenant-agnostic path stays available unchanged.
        access
            .fetch("prov-a", DataRequest::new("k", vec![]))
            .await
            .unwrap();

        let seen = provider.seen.lock().unwrap().clone();
        assert_eq!(
            seen,
            vec![Some(tenant), None],
            "the provider must receive exactly the authoritative tenant the caller scoped, \
             and None when unscoped — never a fabricated or defaulted tenant"
        );
    }

    #[test]
    fn with_tenant_sets_the_authoritative_identity_and_new_defaults_to_none() {
        assert_eq!(DataRequest::new("k", vec![]).tenant, None);
        let tenant = TenantId::new("tenant-z").unwrap();
        assert_eq!(
            DataRequest::new("k", vec![])
                .with_tenant(tenant.clone())
                .tenant,
            Some(tenant)
        );
    }

    #[test]
    fn debug_output_never_contains_raw_key_or_payload() {
        let request = DataRequest::new("secret-kid-123", b"super-sensitive-bytes".to_vec())
            .with_tenant(TenantId::new("tenant-visible").unwrap());
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

        for rendered in [
            &request_debug,
            &response_debug,
            &error_debug,
            &error_display,
        ] {
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
