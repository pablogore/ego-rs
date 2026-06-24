//! Authentication credentials passed to [`super::AuthenticationProvider`].
//!
//! Callers wrap the raw credential material in a [`Credential`] variant
//! before calling `authenticate`. The provider is responsible for extracting
//! and validating the material.

/// A credential presented by a caller for authentication.
///
/// Marked `#[non_exhaustive]` so that new credential types can be added in
/// future releases without breaking existing `match` arms.
///
/// # Note on Anonymous access
///
/// There is no `Anonymous` variant. Callers that permit unauthenticated
/// access should model the optional credential as `Option<Credential>` and
/// apply their own default-policy logic before reaching the provider.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Credential {
    /// A raw Bearer token string (without the `"Bearer "` prefix).
    BearerToken(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bearer_token_stores_value() {
        let c = Credential::BearerToken("tok".into());
        assert!(matches!(c, Credential::BearerToken(ref s) if s == "tok"));
    }

    #[test]
    fn credential_is_clone_and_eq() {
        let a = Credential::BearerToken("abc".into());
        let b = a.clone();
        assert_eq!(a, b);
    }

    #[test]
    fn credential_debug_contains_variant_name() {
        let c = Credential::BearerToken("x".into());
        let s = format!("{c:?}");
        assert!(s.contains("BearerToken"));
    }
}
