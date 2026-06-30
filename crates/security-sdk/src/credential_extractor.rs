//! CredentialExtractor and RequestContext SPIs, plus built-in extractors.
//!
//! Transport adapters implement `RequestContext` for their native request type.
//! Authentication logic depends only on this abstraction (AD-OIDC-011, INV-9).

use base64::{engine::general_purpose::STANDARD, Engine as _};
use ego_domain::auth::{AuthenticationError, Credential};

/// Minimal, transport-neutral view of an incoming request.
///
/// Transport adapters (`AxumRequestContext`, `TonicRequestContext`, etc.)
/// implement this trait to expose request metadata without coupling the
/// authentication pipeline to any concrete HTTP/gRPC type.
pub trait RequestContext: Send + Sync {
    /// Returns the first value of the named HTTP/metadata header, if present.
    fn header(&self, name: &str) -> Option<&str>;
    /// Returns the first value of the named gRPC / message metadata key, if any.
    fn metadata(&self, key: &str) -> Option<&str>;
    /// Returns the first value of the named query parameter, if present.
    fn query_param(&self, name: &str) -> Option<&str>;
}

/// Extracts a [`Credential`] from an incoming request.
///
/// Decouples `AuthenticationInterceptor` from any specific credential scheme.
/// Implementations are in `security-sdk` (Bearer, Basic, ApiKey) or custom
/// crates. The interceptor holds `Arc<dyn CredentialExtractor>`.
pub trait CredentialExtractor: Send + Sync {
    /// Extract a credential from the request.
    ///
    /// Returns:
    /// - `Ok(Some(credential))` — credential found and well-formed.
    /// - `Ok(None)` — no credential present (anonymous pass-through).
    /// - `Err(InvalidToken)` — header present but malformed.
    fn extract(&self, request: &dyn RequestContext) -> Result<Option<Credential>, AuthenticationError>;
}

// ---------------------------------------------------------------------------
// BearerExtractor
// ---------------------------------------------------------------------------

/// Extracts `Authorization: Bearer <token>` credentials.
///
/// The scheme name is case-insensitive (RFC 6750 §2.1). Exactly one SP must
/// separate the scheme from the token; double-space or tab-delimited headers
/// are rejected with `InvalidToken` (out of spec per RFC 7230 §3.2.6).
pub struct BearerExtractor;

impl CredentialExtractor for BearerExtractor {
    fn extract(&self, request: &dyn RequestContext) -> Result<Option<Credential>, AuthenticationError> {
        match request.header("authorization") {
            None => Ok(None),
            Some(val) => {
                // RFC 6750 §2.1: scheme name is case-insensitive.
                // to_ascii_lowercase preserves byte length for pure-ASCII headers (RFC 7230).
                // Use get() instead of direct indexing as a panic defense against any
                // non-ASCII byte that an upstream proxy may inject in violation of the spec.
                let lower = val.to_ascii_lowercase();
                if let Some(token_lower) = lower.strip_prefix("bearer ") {
                    // Recover the token from the original val to preserve casing.
                    // offset = val.len() - token_lower.len() = 7 (len("bearer "))
                    let offset = val.len() - token_lower.len();
                    let token = val.get(offset..).ok_or_else(|| AuthenticationError::InvalidToken(
                        "Authorization header has invalid byte boundary".into(),
                    ))?;
                    // RFC 7230 §3.2.6: exactly one SP separates scheme from token.
                    // A leading space in the recovered token means the header had
                    // double-space ("Bearer  tok") — reject as malformed.
                    if token.starts_with(' ') {
                        return Err(AuthenticationError::InvalidToken(
                            "Authorization header has extra whitespace after Bearer scheme".into(),
                        ));
                    }
                    Ok(Some(Credential::Bearer(token.to_string())))
                } else if lower.starts_with("bearer") {
                    // Header starts with "bearer" but the separator is not a single SP
                    // (e.g. tab, double-space at position 6). Reject as malformed rather
                    // than silently passing Ok(None) — RFC 7230 §3.2.6.
                    Err(AuthenticationError::InvalidToken(
                        "Authorization header uses Bearer scheme with invalid separator".into(),
                    ))
                } else {
                    // Non-Bearer scheme — let the next extractor in the pipeline handle it.
                    Ok(None)
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// BasicExtractor
// ---------------------------------------------------------------------------

/// Extracts `Authorization: Basic <base64(user:pass)>` credentials.
///
/// The scheme name is case-insensitive (RFC 7235 §2.1). Passwords may contain
/// colons — the first colon is the username/password separator (RFC 7617 §2).
pub struct BasicExtractor;

impl CredentialExtractor for BasicExtractor {
    fn extract(&self, request: &dyn RequestContext) -> Result<Option<Credential>, AuthenticationError> {
        match request.header("authorization") {
            None => Ok(None),
            Some(val) => {
                // RFC 7235 §2.1: auth-scheme is case-insensitive.
                let lower = val.to_ascii_lowercase();
                if !lower.starts_with("basic ") {
                    // Not a Basic credential — let other extractors in the pipeline handle it.
                    return Ok(None);
                }
                // "basic " is 6 bytes; use get() as a panic defense against any non-ASCII byte.
                let encoded = val.get(6..).ok_or_else(|| AuthenticationError::InvalidToken(
                    "Authorization header has invalid byte boundary".into(),
                ))?;
                let decoded = base64_decode(encoded)?;
                let text = String::from_utf8(decoded).map_err(|_| {
                    AuthenticationError::InvalidToken("Basic credential is not valid UTF-8".into())
                })?;
                // RFC 7617 §2: first ':' separates user-id from password.
                // Passwords containing colons are valid and preserved.
                let (username, secret) = text.split_once(':').ok_or_else(|| {
                    AuthenticationError::InvalidToken(
                        "Basic credential missing ':' separator".into(),
                    )
                })?;
                Ok(Some(Credential::Basic {
                    username: username.to_string(),
                    secret: secret.to_string(),
                }))
            }
        }
    }
}

// ---------------------------------------------------------------------------
// ApiKeyExtractor
// ---------------------------------------------------------------------------

/// Extracts a bearer token from a custom header (e.g. `X-Api-Key`).
///
/// Returns `Err(InvalidToken)` when the header is present but empty — an empty
/// API key is never well-formed at a trust boundary.
pub struct ApiKeyExtractor {
    /// Name of the header to read the API key from (case-insensitive lookup
    /// is the responsibility of the `RequestContext` implementation).
    pub header_name: String,
}

impl CredentialExtractor for ApiKeyExtractor {
    fn extract(&self, request: &dyn RequestContext) -> Result<Option<Credential>, AuthenticationError> {
        match request.header(&self.header_name) {
            None => Ok(None),
            Some(val) if val.is_empty() => Err(AuthenticationError::InvalidToken(
                "API key header present but value is empty".into(),
            )),
            Some(val) => Ok(Some(Credential::Bearer(val.to_string()))),
        }
    }
}

// ---------------------------------------------------------------------------
// Internal helper: base64 decode (standard alphabet, padding handled by engine)
// ---------------------------------------------------------------------------

fn base64_decode(s: &str) -> Result<Vec<u8>, AuthenticationError> {
    STANDARD.decode(s).map_err(|_| {
        AuthenticationError::InvalidToken("Basic credential is not valid base64".into())
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    struct MockRequestContext {
        headers: HashMap<String, String>,
    }

    impl MockRequestContext {
        fn new(headers: &[(&str, &str)]) -> Self {
            let mut map = HashMap::new();
            for (k, v) in headers {
                map.insert(k.to_lowercase(), v.to_string());
            }
            Self { headers: map }
        }
    }

    impl RequestContext for MockRequestContext {
        fn header(&self, name: &str) -> Option<&str> {
            self.headers.get(&name.to_lowercase()).map(String::as_str)
        }
        fn metadata(&self, _: &str) -> Option<&str> { None }
        fn query_param(&self, _: &str) -> Option<&str> { None }
    }

    // --- BearerExtractor ---

    #[test]
    fn bearer_extractor_parses_bearer_token() {
        let ctx = MockRequestContext::new(&[("authorization", "Bearer tok-123")]);
        let cred = BearerExtractor.extract(&ctx).unwrap().unwrap();
        assert!(matches!(cred, Credential::Bearer(ref t) if t == "tok-123"));
    }

    #[test]
    fn bearer_extractor_returns_none_when_header_absent() {
        let ctx = MockRequestContext::new(&[]);
        assert!(BearerExtractor.extract(&ctx).unwrap().is_none());
    }

    #[test]
    fn bearer_extractor_returns_none_for_non_bearer_header() {
        // Non-Bearer scheme: let pipeline continue to the next extractor.
        let ctx = MockRequestContext::new(&[("authorization", "Token something")]);
        assert!(BearerExtractor.extract(&ctx).unwrap().is_none());
    }

    #[test]
    fn bearer_extractor_returns_none_for_basic_header() {
        // Basic scheme is not Bearer: Ok(None) so BasicExtractor can handle it.
        let ctx = MockRequestContext::new(&[("authorization", "Basic dXNlcjpwYXNz")]);
        assert!(BearerExtractor.extract(&ctx).unwrap().is_none());
    }

    // BLOCKER-1 / WARNING-3: case-insensitive scheme with uppercase token value preserved
    #[test]
    fn bearer_extractor_preserves_token_casing_with_uppercase_scheme() {
        // BEARER scheme (all-caps) + mixed-case token: the token must come out unchanged.
        let ctx = MockRequestContext::new(&[("authorization", "BEARER Tok-ABC123")]);
        let cred = BearerExtractor.extract(&ctx).unwrap().unwrap();
        assert!(
            matches!(cred, Credential::Bearer(ref t) if t == "Tok-ABC123"),
            "token casing must be preserved regardless of scheme casing"
        );
    }

    // BLOCKER-1: double-space is out-of-spec and must return InvalidToken
    #[test]
    fn bearer_extractor_rejects_double_space_separator() {
        let ctx = MockRequestContext::new(&[("authorization", "Bearer  double-space-token")]);
        let err = BearerExtractor.extract(&ctx).unwrap_err();
        assert!(
            matches!(err, AuthenticationError::InvalidToken(_)),
            "double-space after scheme is out-of-spec and must be rejected"
        );
    }

    // BLOCKER-1: tab separator is not a valid SP per RFC 7230 §3.2.6
    #[test]
    fn bearer_extractor_rejects_tab_separator() {
        let ctx = MockRequestContext::new(&[("authorization", "Bearer\ttabbed-token")]);
        let err = BearerExtractor.extract(&ctx).unwrap_err();
        assert!(
            matches!(err, AuthenticationError::InvalidToken(_)),
            "tab separator is not a valid SP and must be rejected"
        );
    }

    // --- BasicExtractor ---

    #[test]
    fn basic_extractor_is_case_insensitive_on_scheme() {
        for scheme in &["Basic", "basic", "BASIC"] {
            let header = format!("{scheme} dXNlcjpwYXNz");
            let ctx = MockRequestContext::new(&[("authorization", &header)]);
            let cred = BasicExtractor.extract(&ctx).unwrap().unwrap();
            match cred {
                Credential::Basic { username, secret } => {
                    assert_eq!(username, "user", "scheme={scheme}");
                    assert_eq!(secret, "pass", "scheme={scheme}");
                }
                _ => panic!("expected Basic for scheme={scheme}"),
            }
        }
    }

    #[test]
    fn basic_extractor_decodes_user_pass() {
        // "user:pass" in base64 = "dXNlcjpwYXNz"
        let ctx = MockRequestContext::new(&[("authorization", "Basic dXNlcjpwYXNz")]);
        let cred = BasicExtractor.extract(&ctx).unwrap().unwrap();
        match cred {
            Credential::Basic { username, secret } => {
                assert_eq!(username, "user");
                assert_eq!(secret, "pass");
            }
            _ => panic!("expected Basic"),
        }
    }

    // HIGH-5 / BLOCKER-2: password containing colon — split on first colon only (RFC 7617 §2)
    #[test]
    fn basic_extractor_password_with_colon_splits_on_first_colon() {
        // "user:pass:word" in standard base64 = "dXNlcjpwYXNzOndvcmQ="
        // Verified: python3 -c "import base64; print(base64.b64encode(b'user:pass:word').decode())"
        let header = "Basic dXNlcjpwYXNzOndvcmQ=";
        let ctx = MockRequestContext::new(&[("authorization", header)]);
        let cred = BasicExtractor.extract(&ctx).unwrap().unwrap();
        match cred {
            Credential::Basic { username, secret } => {
                assert_eq!(username, "user");
                assert_eq!(secret, "pass:word", "password-with-colon must be preserved after first ':'");
            }
            _ => panic!("expected Basic"),
        }
    }

    // BLOCKER-2: empty encoded token ("Basic " with no token bytes)
    #[test]
    fn basic_extractor_rejects_empty_token() {
        // "Basic " with nothing after — decodes to empty bytes, no colon
        let ctx = MockRequestContext::new(&[("authorization", "Basic ")]);
        // Empty base64 decodes to [], then no colon → InvalidToken
        let result = BasicExtractor.extract(&ctx);
        // Either Err(InvalidToken) or decoding fails — must not return Ok
        assert!(result.is_err(), "empty basic token must be rejected");
    }

    #[test]
    fn basic_extractor_returns_none_when_header_absent() {
        let ctx = MockRequestContext::new(&[]);
        assert!(BasicExtractor.extract(&ctx).unwrap().is_none());
    }

    #[test]
    fn basic_extractor_returns_err_for_invalid_base64() {
        let ctx = MockRequestContext::new(&[("authorization", "Basic !!!not_base64!!!")]);
        let err = BasicExtractor.extract(&ctx).unwrap_err();
        assert!(matches!(err, AuthenticationError::InvalidToken(_)));
    }

    // B-1: non-Basic scheme must return Ok(None) so other extractors in the pipeline can handle it
    #[test]
    fn basic_extractor_returns_none_for_non_basic_scheme() {
        let ctx = MockRequestContext::new(&[("authorization", "Bearer tok-123")]);
        assert!(BasicExtractor.extract(&ctx).unwrap().is_none());
    }

    // --- ApiKeyExtractor ---

    #[test]
    fn api_key_extractor_reads_configured_header() {
        let ctx = MockRequestContext::new(&[("x-api-key", "my-secret-key")]);
        let extractor = ApiKeyExtractor { header_name: "x-api-key".into() };
        let cred = extractor.extract(&ctx).unwrap().unwrap();
        assert!(matches!(cred, Credential::Bearer(ref t) if t == "my-secret-key"));
    }

    #[test]
    fn api_key_extractor_returns_none_when_header_absent() {
        let ctx = MockRequestContext::new(&[]);
        let extractor = ApiKeyExtractor { header_name: "x-api-key".into() };
        assert!(extractor.extract(&ctx).unwrap().is_none());
    }

    // SUGGESTION-2: empty API key header value must be rejected (not Ok(Some(Bearer(""))))
    #[test]
    fn api_key_extractor_rejects_empty_header_value() {
        let ctx = MockRequestContext::new(&[("x-api-key", "")]);
        let extractor = ApiKeyExtractor { header_name: "x-api-key".into() };
        let err = extractor.extract(&ctx).unwrap_err();
        assert!(
            matches!(err, AuthenticationError::InvalidToken(_)),
            "empty API key must be InvalidToken, not a well-formed credential"
        );
    }

    // --- Send + Sync compile-time assertions ---

    fn assert_send_sync<T: Send + Sync>() {}

    #[test]
    fn extractors_are_send_sync() {
        assert_send_sync::<BearerExtractor>();
        assert_send_sync::<BasicExtractor>();
        assert_send_sync::<ApiKeyExtractor>();
    }
}
