//! OIDC provider configuration types.
//!
//! All URL fields use `url::Url` — validated at construction time (INV-10).
//! `OidcProviderConfig::validate()` is called by each provider constructor.

use std::collections::HashMap;

use ego_domain::auth::AuthenticationError;

/// Controls how the provider decides whether a token is a JWT or opaque.
#[derive(Debug, Clone, serde::Deserialize)]
pub enum TokenFormat {
    /// Always validate as JWT.
    Jwt,
    /// Always use introspection.
    Opaque,
    /// Detect by format: two base64url segments separated by dots = JWT; else opaque.
    Auto,
}

/// Full OIDC provider configuration. Derives `Deserialize` for kit-config integration.
///
/// Either `issuer_url` or `jwks_uri` MUST be present (validated by `validate()`).
///
/// `Debug` output redacts `introspection_client_secret` to prevent accidental secret exposure
/// in logs or error messages.
#[derive(Clone, Default, serde::Deserialize)]
pub struct OidcProviderConfig {
    /// Issuer URL. Used for OIDC Discovery when `jwks_uri` is absent.
    pub issuer_url: Option<url::Url>,
    /// JWKS URI. Takes precedence over `issuer_url` when both are set.
    pub jwks_uri: Option<url::Url>,
    /// Expected `iss` claim value.
    pub expected_iss: Option<String>,
    /// Expected `aud` claim values (at least one must match).
    pub expected_aud: Option<Vec<String>>,
    /// Leeway in seconds applied to `exp` and `nbf` checks. Default 0.
    ///
    /// Tokens expired by fewer than this many seconds are still accepted.
    /// Use small values (≤ 30s). This is NOT symmetric clock-skew — only `exp`/`nbf` are affected.
    pub leeway_seconds: Option<u64>,
    /// JWKS background refresh interval in seconds. Default 300.
    pub jwks_refresh_ttl_seconds: Option<u64>,
    /// Token format detection mode. Default `Auto`.
    pub token_format: Option<TokenFormat>,
    /// Introspection endpoint URL. Required for opaque token support.
    /// Must be HTTPS unless host is localhost/127.0.0.1 (INV-11).
    pub introspection_endpoint: Option<url::Url>,
    /// Client ID for introspection endpoint authentication.
    pub introspection_client_id: Option<String>,
    /// Client secret for introspection endpoint authentication.
    pub introspection_client_secret: Option<String>,
    /// Introspection response cache TTL in seconds. `None` = cache disabled (default).
    pub introspection_cache_ttl_seconds: Option<u64>,
    /// Algorithms accepted on the JWT path. If `None`, defaults to `[Rs256, Es256]`.
    pub allowed_algorithms: Option<Vec<crate::config::JwtAlgorithm>>,
}

impl std::fmt::Debug for OidcProviderConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OidcProviderConfig")
            .field("issuer_url", &self.issuer_url)
            .field("jwks_uri", &self.jwks_uri)
            .field("expected_iss", &self.expected_iss)
            .field("expected_aud", &self.expected_aud)
            .field("leeway_seconds", &self.leeway_seconds)
            .field("jwks_refresh_ttl_seconds", &self.jwks_refresh_ttl_seconds)
            .field("token_format", &self.token_format)
            .field("introspection_endpoint", &self.introspection_endpoint)
            .field("introspection_client_id", &self.introspection_client_id)
            .field(
                "introspection_client_secret",
                &self
                    .introspection_client_secret
                    .as_ref()
                    .map(|_| "[REDACTED]"),
            )
            .field(
                "introspection_cache_ttl_seconds",
                &self.introspection_cache_ttl_seconds,
            )
            .field("allowed_algorithms", &self.allowed_algorithms)
            .finish()
    }
}

impl OidcProviderConfig {
    /// Validate the configuration.
    ///
    /// Returns `Err(ProviderUnavailable)` when:
    /// - Both `issuer_url` and `jwks_uri` are absent.
    /// - `jwks_uri`, `issuer_url`, or `introspection_endpoint` uses `http://` with a non-localhost host (INV-11).
    pub fn validate(&self) -> Result<(), AuthenticationError> {
        if self.issuer_url.is_none() && self.jwks_uri.is_none() {
            return Err(AuthenticationError::ProviderUnavailable(
                "either issuer_url or jwks_uri must be configured".into(),
            ));
        }

        if let Some(uri) = &self.jwks_uri {
            validate_url_requires_https(uri, "jwks_uri")?;
        }
        if let Some(uri) = &self.issuer_url {
            validate_url_requires_https(uri, "issuer_url")?;
        }

        if let Some(ep) = &self.introspection_endpoint {
            validate_url_requires_https(ep, "introspection_endpoint")?;
        }

        if self.introspection_endpoint.is_some()
            && (self.introspection_client_id.is_none()
                || self.introspection_client_secret.is_none())
        {
            return Err(AuthenticationError::ProviderUnavailable(
                "introspection_endpoint requires introspection_client_id and introspection_client_secret"
                    .to_string(),
            ));
        }

        if self.introspection_endpoint.is_none()
            && (self.introspection_client_id.is_some()
                || self.introspection_client_secret.is_some())
        {
            return Err(AuthenticationError::ProviderUnavailable(
                "introspection_client_id/secret set without introspection_endpoint".to_string(),
            ));
        }

        // H-3: cap cache TTL to prevent long-lived tokens defeating revocation.
        // Also reject ttl=0 — a zero TTL disables caching by timing out instantly;
        // callers who want no cache should use None, not 0.
        const MAX_INTROSPECTION_CACHE_TTL_SECS: u64 = 300;
        if let Some(ttl) = self.introspection_cache_ttl_seconds {
            if ttl == 0 {
                return Err(AuthenticationError::ProviderUnavailable(
                    "introspection_cache_ttl_seconds must be >= 1 (use None to disable cache)"
                        .into(),
                ));
            }
            if ttl > MAX_INTROSPECTION_CACHE_TTL_SECS {
                return Err(AuthenticationError::ProviderUnavailable(format!(
                    "introspection_cache_ttl_seconds must be <= {MAX_INTROSPECTION_CACHE_TTL_SECS} \
                     (longer TTLs defeat token revocation)"
                )));
            }
        }

        // H-5: empty allowed_algorithms list is invalid — it would reject all tokens.
        if let Some(algs) = &self.allowed_algorithms {
            if algs.is_empty() {
                return Err(AuthenticationError::ProviderUnavailable(
                    "allowed_algorithms must not be empty".into(),
                ));
            }
        }

        // R1-B1: when jwks_uri is configured, expected_iss is always required regardless of
        // whether issuer_url is also present. Without expected_iss the iss claim is not validated
        // and tokens from any issuer are accepted (W3: closes the bypass where both urls are set).
        if self.jwks_uri.is_some() && self.expected_iss.is_none() {
            return Err(AuthenticationError::ProviderUnavailable(
                "expected_iss is required when jwks_uri is configured — \
                 without it, tokens from any issuer are accepted"
                    .into(),
            ));
        }

        // R2-W2: discovery-only path (issuer_url set, jwks_uri absent, expected_iss absent).
        // Not a hard error — operators may set up OIDC discovery before they know the issuer
        // string. Advisory warning only; the iss claim will not be validated at runtime.
        if self.issuer_url.is_some() && self.jwks_uri.is_none() && self.expected_iss.is_none() {
            tracing::warn!(
                "OidcProviderConfig: expected_iss not set — iss claim in JWT will not be \
                 validated. Set expected_iss to prevent issuer confusion attacks."
            );
        }

        Ok(())
    }
}

/// Returns `Err` when the URL does not use `https://` (or `http://` with a localhost host).
/// Any scheme other than `https` or `http` is also rejected. The `field` name is included
/// in the error message for diagnostics (H-6).
pub(crate) fn validate_url_requires_https(
    url: &url::Url,
    field: &str,
) -> Result<(), AuthenticationError> {
    match url.scheme() {
        "https" => Ok(()),
        "http" => {
            let host = url.host_str().unwrap_or("");
            // url::Url::host_str() returns "[::1]" WITH brackets for IPv6 literals (WHATWG URL spec).
            if host == "localhost" || host == "127.0.0.1" || host == "[::1]" {
                Ok(())
            } else {
                Err(AuthenticationError::ProviderUnavailable(format!(
                    "{field} must use https (http allowed only for localhost/127.0.0.1/[::1])"
                )))
            }
        }
        scheme => Err(AuthenticationError::ProviderUnavailable(format!(
            "{field} has unsupported scheme '{scheme}' — must be https"
        ))),
    }
}

/// Configuration for multi-issuer routing.
///
/// Map of issuer string → `OidcProviderConfig`.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct MultiIssuerConfig {
    /// Map of issuer URL string to per-issuer configuration.
    pub issuers: HashMap<String, OidcProviderConfig>,
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn url(s: &str) -> url::Url {
        url::Url::parse(s).unwrap()
    }

    fn cfg_with_jwks_uri() -> OidcProviderConfig {
        OidcProviderConfig {
            issuer_url: None,
            jwks_uri: Some(url("https://example.com/.well-known/jwks.json")),
            // R1-B1: expected_iss required when jwks_uri set without issuer_url.
            expected_iss: Some("https://example.com".into()),
            expected_aud: None,
            leeway_seconds: None,
            jwks_refresh_ttl_seconds: None,
            token_format: None,
            introspection_endpoint: None,
            introspection_client_id: None,
            introspection_client_secret: None,
            introspection_cache_ttl_seconds: None,
            allowed_algorithms: None,
        }
    }

    fn cfg_with_neither() -> OidcProviderConfig {
        OidcProviderConfig {
            issuer_url: None,
            jwks_uri: None,
            ..cfg_with_jwks_uri()
        }
    }

    // --- validate() ---

    #[test]
    fn validate_returns_ok_when_jwks_uri_is_set() {
        assert!(cfg_with_jwks_uri().validate().is_ok());
    }

    #[test]
    fn validate_returns_ok_when_issuer_url_is_set() {
        // Base from Default::default() so expected_iss is None — tests discovery-only path.
        let cfg = OidcProviderConfig {
            issuer_url: Some(url("https://example.com")),
            ..Default::default()
        };
        assert!(cfg.validate().is_ok());
    }

    // R2-W2: discovery-only path with expected_iss = None must return Ok (not a hard error).
    // A warn! is emitted at runtime but validate() must not reject this config — operators may
    // legitimately use OIDC discovery before they know the exact issuer string.
    #[test]
    fn validate_returns_ok_when_issuer_url_set_without_expected_iss() {
        let cfg = OidcProviderConfig {
            issuer_url: Some(url("https://example.com")),
            jwks_uri: None,
            expected_iss: None, // explicit: discovery path with no issuer validation
            ..Default::default()
        };
        assert!(
            cfg.validate().is_ok(),
            "discovery-only path without expected_iss must be Ok (advisory warn, not hard error)"
        );
    }

    #[test]
    fn validate_returns_provider_unavailable_when_neither_url_is_set() {
        let err = cfg_with_neither().validate().unwrap_err();
        assert!(matches!(err, AuthenticationError::ProviderUnavailable(_)));
    }

    #[test]
    fn validate_returns_ok_when_both_are_set() {
        let mut cfg = cfg_with_jwks_uri();
        cfg.issuer_url = Some(url("https://example.com"));
        assert!(cfg.validate().is_ok());
        assert!(
            cfg.jwks_uri.is_some(),
            "jwks_uri field is Some when both set"
        );
    }

    // --- INV-11: introspection endpoint ---

    #[test]
    fn validate_rejects_http_non_localhost_introspection_endpoint() {
        let mut cfg = cfg_with_jwks_uri();
        cfg.introspection_endpoint = Some(url("http://idp.example.com/introspect"));
        let err = cfg.validate().unwrap_err();
        assert!(matches!(err, AuthenticationError::ProviderUnavailable(_)));
    }

    #[test]
    fn validate_accepts_https_introspection_endpoint() {
        let mut cfg = cfg_with_jwks_uri();
        cfg.introspection_endpoint = Some(url("https://idp.example.com/introspect"));
        cfg.introspection_client_id = Some("cid".into());
        cfg.introspection_client_secret = Some("csec".into());
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn validate_accepts_http_localhost_introspection_endpoint() {
        let mut cfg = cfg_with_jwks_uri();
        cfg.introspection_endpoint = Some(url("http://localhost:8080/introspect"));
        cfg.introspection_client_id = Some("cid".into());
        cfg.introspection_client_secret = Some("csec".into());
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn validate_accepts_http_127_0_0_1_introspection_endpoint() {
        let mut cfg = cfg_with_jwks_uri();
        cfg.introspection_endpoint = Some(url("http://127.0.0.1:8080/introspect"));
        cfg.introspection_client_id = Some("cid".into());
        cfg.introspection_client_secret = Some("csec".into());
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn validate_rejects_introspection_endpoint_without_credentials() {
        let mut cfg = cfg_with_jwks_uri();
        cfg.introspection_endpoint = Some(url("https://idp.example.com/introspect"));
        // no client_id / client_secret
        let err = cfg.validate().unwrap_err();
        assert!(matches!(err, AuthenticationError::ProviderUnavailable(_)));
    }

    #[test]
    fn validate_rejects_jwt_format_introspection_endpoint_without_credentials() {
        let mut cfg = cfg_with_jwks_uri();
        cfg.token_format = Some(TokenFormat::Jwt);
        cfg.introspection_endpoint = Some(url("https://idp.example.com/introspect"));
        // no client_id / client_secret — must fail regardless of token_format
        let err = cfg.validate().unwrap_err();
        assert!(matches!(err, AuthenticationError::ProviderUnavailable(_)));
    }

    // --- orphaned introspection credentials ---

    #[test]
    fn validate_rejects_orphaned_introspection_client_id() {
        // expected_iss is required by R1-B1; set it so the orphaned-credential guard is reached.
        let config = OidcProviderConfig {
            jwks_uri: Some(url("https://example.com/.well-known/jwks.json")),
            expected_iss: Some("https://example.com".into()),
            introspection_client_id: Some("orphaned-id".to_string()),
            ..Default::default()
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn validate_rejects_orphaned_introspection_client_secret() {
        // expected_iss is required by R1-B1; set it so the orphaned-credential guard is reached.
        let config = OidcProviderConfig {
            jwks_uri: Some(url("https://example.com/.well-known/jwks.json")),
            expected_iss: Some("https://example.com".into()),
            introspection_client_secret: Some("orphaned-secret".to_string()),
            ..Default::default()
        };
        assert!(config.validate().is_err());
    }

    // HIGH-1: both client_id AND client_secret without endpoint — common operator mistake
    #[test]
    fn validate_rejects_both_orphaned_credentials_without_endpoint() {
        // expected_iss is required by R1-B1; set it so the orphaned-credential guard is reached.
        let config = OidcProviderConfig {
            jwks_uri: Some(url("https://example.com/.well-known/jwks.json")),
            expected_iss: Some("https://example.com".into()),
            introspection_client_id: Some("cid".to_string()),
            introspection_client_secret: Some("csecret".to_string()),
            ..Default::default()
        };
        let err = config.validate().unwrap_err();
        assert!(
            matches!(err, AuthenticationError::ProviderUnavailable(_)),
            "both credentials without endpoint must be rejected"
        );
    }

    // HIGH-4: OidcProviderConfig Debug must redact introspection_client_secret
    #[test]
    fn debug_output_redacts_introspection_client_secret() {
        let config = OidcProviderConfig {
            jwks_uri: Some(url("https://example.com/jwks")),
            introspection_client_secret: Some("super-secret-value".to_string()),
            ..Default::default()
        };
        let debug_str = format!("{config:?}");
        assert!(
            !debug_str.contains("super-secret-value"),
            "Debug output must not expose the client secret; got: {debug_str}"
        );
        assert!(
            debug_str.contains("[REDACTED]"),
            "Debug output must show [REDACTED] for secret; got: {debug_str}"
        );
    }

    // B-1: http:// for jwks_uri is now rejected by validate() (HTTPS enforcement).
    #[test]
    fn validate_rejects_http_jwks_uri() {
        let config = OidcProviderConfig {
            jwks_uri: Some(url("http://example.com/jwks")),
            ..Default::default()
        };
        let err = config.validate().unwrap_err();
        assert!(matches!(err, AuthenticationError::ProviderUnavailable(_)));
    }

    // B-1: http:// for issuer_url is rejected by validate() (HTTPS enforcement).
    #[test]
    fn validate_rejects_http_issuer_url() {
        let config = OidcProviderConfig {
            issuer_url: Some(url("http://example.com")),
            ..Default::default()
        };
        let err = config.validate().unwrap_err();
        assert!(matches!(err, AuthenticationError::ProviderUnavailable(_)));
    }

    // H-2/SUGGESTION-1: IPv6 loopback [::1] must be allowed for http introspection endpoint.
    #[test]
    fn validate_accepts_http_ipv6_loopback_introspection() {
        let mut cfg = cfg_with_jwks_uri();
        cfg.introspection_endpoint = Some(url("http://[::1]:8080/introspect"));
        cfg.introspection_client_id = Some("cid".into());
        cfg.introspection_client_secret = Some("csec".into());
        assert!(
            cfg.validate().is_ok(),
            "::1 loopback must be allowed for http introspection"
        );
    }

    // H-3: introspection_cache_ttl_seconds above 300 must be rejected.
    #[test]
    fn validate_rejects_introspection_cache_ttl_above_max() {
        let mut cfg = cfg_with_jwks_uri();
        cfg.introspection_endpoint = Some(url("https://idp.example.com/introspect"));
        cfg.introspection_client_id = Some("cid".into());
        cfg.introspection_client_secret = Some("csec".into());
        cfg.introspection_cache_ttl_seconds = Some(301);
        let err = cfg.validate().unwrap_err();
        assert!(
            matches!(err, AuthenticationError::ProviderUnavailable(_)),
            "TTL > 300 must be ProviderUnavailable"
        );
    }

    #[test]
    fn validate_accepts_introspection_cache_ttl_at_max() {
        let mut cfg = cfg_with_jwks_uri();
        cfg.introspection_endpoint = Some(url("https://idp.example.com/introspect"));
        cfg.introspection_client_id = Some("cid".into());
        cfg.introspection_client_secret = Some("csec".into());
        cfg.introspection_cache_ttl_seconds = Some(300);
        assert!(cfg.validate().is_ok(), "TTL == 300 must be accepted");
    }

    // H-3: TTL=0 is rejected (use None to disable cache, not zero).
    #[test]
    fn validate_rejects_introspection_cache_ttl_zero() {
        let mut cfg = cfg_with_jwks_uri();
        cfg.introspection_endpoint = Some(url("https://idp.example.com/introspect"));
        cfg.introspection_client_id = Some("cid".into());
        cfg.introspection_client_secret = Some("csec".into());
        cfg.introspection_cache_ttl_seconds = Some(0);
        let err = cfg.validate().unwrap_err();
        assert!(
            matches!(err, AuthenticationError::ProviderUnavailable(_)),
            "TTL == 0 must be ProviderUnavailable"
        );
    }

    // H-5: empty allowed_algorithms must be rejected.
    #[test]
    fn validate_rejects_empty_allowed_algorithms() {
        let mut cfg = cfg_with_jwks_uri();
        cfg.allowed_algorithms = Some(vec![]);
        let err = cfg.validate().unwrap_err();
        assert!(
            matches!(err, AuthenticationError::ProviderUnavailable(_)),
            "empty allowed_algorithms must be ProviderUnavailable"
        );
    }

    // H-6: non-http/https scheme must be rejected.
    #[test]
    fn validate_rejects_file_scheme_jwks_uri() {
        let config = OidcProviderConfig {
            jwks_uri: Some(url("file:///etc/passwd")),
            ..Default::default()
        };
        let err = config.validate().unwrap_err();
        assert!(
            matches!(err, AuthenticationError::ProviderUnavailable(_)),
            "file:// scheme must be rejected"
        );
    }

    // --- TokenFormat serde ---

    #[test]
    fn token_format_deserializes_auto() {
        let v: TokenFormat = serde_json::from_str("\"Auto\"").unwrap();
        assert!(matches!(v, TokenFormat::Auto));
    }

    #[test]
    fn token_format_deserializes_jwt() {
        let v: TokenFormat = serde_json::from_str("\"Jwt\"").unwrap();
        assert!(matches!(v, TokenFormat::Jwt));
    }

    #[test]
    fn token_format_deserializes_opaque() {
        let v: TokenFormat = serde_json::from_str("\"Opaque\"").unwrap();
        assert!(matches!(v, TokenFormat::Opaque));
    }
    // M-3: non-loopback IPv6 HTTP must be rejected — only ::1 is a loopback exemption.
    #[test]
    fn validate_rejects_http_non_loopback_ipv6_introspection() {
        let mut cfg = cfg_with_jwks_uri();
        cfg.introspection_endpoint = Some(url("http://[2001:db8::1]/introspect"));
        cfg.introspection_client_id = Some("cid".into());
        cfg.introspection_client_secret = Some("csec".into());
        let err = cfg.validate().unwrap_err();
        assert!(
            matches!(err, AuthenticationError::ProviderUnavailable(_)),
            "http with non-loopback IPv6 must be rejected, got: {err:?}"
        );
    }
}
