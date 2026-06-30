//! OIDC discovery — `DiscoveryProvider` (pub(crate)), `HttpDiscoveryProvider` (pub(crate)),
//! and `OidcEndpoints` (pub — callers receive it as the result of discovery).
//!
//! `DiscoveryProvider` is an internal implementation detail (AD-OIDC-003).
//! Custom discovery strategies are not a supported extension point.

use async_trait::async_trait;
use ego_domain::auth::AuthenticationError;
use tracing::warn;

use crate::oidc_config::validate_url_requires_https;

// ---------------------------------------------------------------------------
// OidcEndpoints — public (callers inspect the discovery result)
// ---------------------------------------------------------------------------

/// Minimal OIDC discovery result.
///
/// Only the endpoints needed for token validation are exposed — no other
/// discovery document fields are part of the public API (INV-2).
pub struct OidcEndpoints {
    /// JWKS URI — always present in a conforming OIDC discovery document.
    pub jwks_uri: url::Url,
    /// RFC 7662 introspection endpoint, if the IdP advertises it.
    pub introspection_endpoint: Option<url::Url>,
}

// ---------------------------------------------------------------------------
// OidcConfiguration — internal serde type (pub(crate))
// ---------------------------------------------------------------------------

/// Internal representation of the OIDC discovery document (subset).
#[derive(serde::Deserialize)]
pub(crate) struct OidcConfiguration {
    pub jwks_uri: Option<url::Url>,
    pub introspection_endpoint: Option<url::Url>,
}

// ---------------------------------------------------------------------------
// DiscoveryProvider — pub(crate) SPI
// ---------------------------------------------------------------------------

/// Fetches OIDC discovery configuration for a given issuer URL.
///
/// `pub(crate)` — internal implementation detail, not a public SPI.
/// Custom discovery strategies are not a supported extension point.
#[async_trait]
pub(crate) trait DiscoveryProvider: Send + Sync {
    async fn fetch_configuration(
        &self,
        issuer_url: &url::Url,
    ) -> Result<OidcEndpoints, AuthenticationError>;
}

// ---------------------------------------------------------------------------
// HttpDiscoveryProvider — pub(crate)
// ---------------------------------------------------------------------------

/// Default `DiscoveryProvider` backed by `reqwest`.
pub(crate) struct HttpDiscoveryProvider {
    client: reqwest::Client,
}

impl HttpDiscoveryProvider {
    pub(crate) fn new() -> Self {
        Self {
            client: reqwest::Client::builder()
                .use_rustls_tls()
                .timeout(std::time::Duration::from_secs(10))
                .build()
                .expect("failed to build reqwest client"),
        }
    }
}

#[async_trait]
impl DiscoveryProvider for HttpDiscoveryProvider {
    async fn fetch_configuration(
        &self,
        issuer_url: &url::Url,
    ) -> Result<OidcEndpoints, AuthenticationError> {
        // Append `/.well-known/openid-configuration` to the issuer URL.
        // url::Url::join strips the last path segment when the base has no trailing
        // slash, so ensure there is one before joining (S-B01).
        let base = if issuer_url.as_str().ends_with('/') {
            issuer_url.clone()
        } else {
            url::Url::parse(&format!("{}/", issuer_url)).map_err(|_| {
                AuthenticationError::ProviderUnavailable("invalid issuer URL".to_string())
            })?
        };
        let discovery_url = base
            .join(".well-known/openid-configuration")
            .map_err(|_| {
                AuthenticationError::ProviderUnavailable(
                    "failed to build discovery URL".to_string(),
                )
            })?;

        let resp = self
            .client
            .get(discovery_url.as_str())
            .send()
            .await
            .map_err(|e| {
                warn!("OIDC discovery HTTP error: {e}");
                AuthenticationError::ProviderUnavailable(format!("discovery HTTP error: {e}"))
            })?;

        if !resp.status().is_success() {
            return Err(AuthenticationError::ProviderUnavailable(format!(
                "discovery returned HTTP {}",
                resp.status()
            )));
        }

        let config: OidcConfiguration = resp.json().await.map_err(|e| {
            AuthenticationError::ProviderUnavailable(format!("discovery parse error: {e}"))
        })?;

        // OQ-5: jwks_uri is REQUIRED (OIDC Core §3 — absent = non-compliant IdP).
        let jwks_uri = config.jwks_uri.ok_or_else(|| {
            AuthenticationError::ProviderUnavailable(
                "discovery document missing required jwks_uri".into(),
            )
        })?;

        // Validate that URLs from the discovery document also satisfy the HTTPS requirement.
        // A compromised IdP could advertise http:// endpoints — reject them the same way
        // statically configured URLs are rejected (INV-11).
        validate_url_requires_https(&jwks_uri, "jwks_uri (from discovery)")?;
        if let Some(ref ep) = config.introspection_endpoint {
            validate_url_requires_https(ep, "introspection_endpoint (from discovery)")?;
        }

        Ok(OidcEndpoints { jwks_uri, introspection_endpoint: config.introspection_endpoint })
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // Inline FakeDiscovery for unit tests — full version in test_kit (T-14).
    pub(crate) struct FakeDiscovery {
        endpoints: OidcEndpoints,
    }

    impl FakeDiscovery {
        pub(crate) fn new(jwks_uri: url::Url) -> Self {
            Self { endpoints: OidcEndpoints { jwks_uri, introspection_endpoint: None } }
        }
    }

    #[async_trait]
    impl DiscoveryProvider for FakeDiscovery {
        async fn fetch_configuration(
            &self,
            _: &url::Url,
        ) -> Result<OidcEndpoints, AuthenticationError> {
            Ok(OidcEndpoints {
                jwks_uri: self.endpoints.jwks_uri.clone(),
                introspection_endpoint: self.endpoints.introspection_endpoint.clone(),
            })
        }
    }

    #[test]
    fn oidc_endpoints_has_only_jwks_uri_and_introspection() {
        // Structural check: the type has exactly two fields.
        let ep = OidcEndpoints {
            jwks_uri: url::Url::parse("https://example.com/jwks").unwrap(),
            introspection_endpoint: None,
        };
        assert_eq!(ep.jwks_uri.as_str(), "https://example.com/jwks");
        assert!(ep.introspection_endpoint.is_none());
    }

    #[test]
    fn oidc_configuration_is_crate_private() {
        // OidcConfiguration should not be accessible outside the crate.
        // This test confirms it compiles with pub(crate) access.
        let json = r#"{"jwks_uri": "https://example.com/jwks"}"#;
        let cfg: OidcConfiguration = serde_json::from_str(json).unwrap();
        assert!(cfg.jwks_uri.is_some());
    }

    #[test]
    fn oidc_configuration_with_null_jwks_uri_deserializes() {
        let json = r#"{}"#;
        let cfg: OidcConfiguration = serde_json::from_str(json).unwrap();
        assert!(cfg.jwks_uri.is_none());
    }

    #[test]
    fn fake_discovery_returns_configured_endpoints() {
        let jwks_uri = url::Url::parse("https://example.com/jwks").unwrap();
        let discovery = FakeDiscovery::new(jwks_uri.clone());
        let endpoints = futures_executor::block_on(discovery.fetch_configuration(
            &url::Url::parse("https://example.com").unwrap(),
        ))
        .unwrap();
        assert_eq!(endpoints.jwks_uri, jwks_uri);
    }
}
