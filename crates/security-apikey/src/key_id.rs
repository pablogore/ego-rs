//! [`ApiKeyId`] — validated API key identifier value object.

use ego_domain::auth::AuthenticationError;

/// A validated API key identifier.
///
/// Non-empty, composed exclusively of `[a-zA-Z0-9_-]`, maximum 128 characters.
/// Suitable as a [`std::collections::HashMap`] key.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ApiKeyId(String);

impl ApiKeyId {
    /// Maximum allowed byte length for an `ApiKeyId`.
    pub const MAX_LEN: usize = 128;

    /// Constructs an `ApiKeyId`, validating charset and length.
    ///
    /// # Errors
    /// Returns [`AuthenticationError::InvalidToken`] if `s` is empty,
    /// contains a character outside `[a-zA-Z0-9_-]`, or exceeds 128 characters.
    pub fn new(s: &str) -> Result<Self, AuthenticationError> {
        if s.is_empty() {
            return Err(AuthenticationError::InvalidToken(
                "api key id must not be empty".into(),
            ));
        }
        if s.len() > Self::MAX_LEN {
            return Err(AuthenticationError::InvalidToken(
                "api key id exceeds maximum length".into(),
            ));
        }
        if !s
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        {
            return Err(AuthenticationError::InvalidToken(
                "api key id contains forbidden characters".into(),
            ));
        }
        Ok(Self(s.to_owned()))
    }

    /// Returns the inner string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_alphanumeric_id_accepted() {
        let id = ApiKeyId::new("my-key_01").unwrap();
        assert_eq!(id.as_str(), "my-key_01");
    }

    #[test]
    fn valid_uuid_style_id_accepted() {
        let id = ApiKeyId::new("abc123-ABC_XYZ").unwrap();
        assert_eq!(id.as_str(), "abc123-ABC_XYZ");
    }

    #[test]
    fn empty_string_rejected() {
        assert!(matches!(
            ApiKeyId::new(""),
            Err(AuthenticationError::InvalidToken(_))
        ));
    }

    #[test]
    fn at_sign_forbidden() {
        assert!(matches!(
            ApiKeyId::new("bad@key"),
            Err(AuthenticationError::InvalidToken(_))
        ));
    }

    #[test]
    fn dot_forbidden() {
        assert!(matches!(
            ApiKeyId::new("bad.key"),
            Err(AuthenticationError::InvalidToken(_))
        ));
    }

    #[test]
    fn space_forbidden() {
        assert!(matches!(
            ApiKeyId::new("bad key"),
            Err(AuthenticationError::InvalidToken(_))
        ));
    }

    #[test]
    fn exactly_128_chars_accepted() {
        let s: String = "a".repeat(128);
        let id = ApiKeyId::new(&s).unwrap();
        assert_eq!(id.as_str().len(), 128);
    }

    #[test]
    fn exactly_129_chars_rejected() {
        let s: String = "a".repeat(129);
        assert!(matches!(
            ApiKeyId::new(&s),
            Err(AuthenticationError::InvalidToken(_))
        ));
    }
}
