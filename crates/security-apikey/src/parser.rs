//! [`ApiKeyParser`] trait and [`DefaultApiKeyParser`] implementation.

use ego_domain::auth::AuthenticationError;

use crate::key_id::ApiKeyId;
use crate::secret::Secret;

/// Parses a raw credential string into an `(ApiKeyId, Secret)` pair.
///
/// Implementations MUST be deterministic: the same `raw` input always produces
/// the same result. Malformed input returns [`AuthenticationError::InvalidToken`].
pub trait ApiKeyParser: Send + Sync {
    /// Parses `raw` into a key identifier and secret.
    ///
    /// # Errors
    /// Returns [`AuthenticationError::InvalidToken`] for any malformed input.
    fn parse(&self, raw: &str) -> Result<(ApiKeyId, Secret), AuthenticationError>;
}

/// Default parser: splits on the **first** `.` separator.
///
/// Format: `{key_id}.{secret}` — the secret may contain further dots;
/// only the first is treated as a separator.
///
/// Empty key-id or empty secret halves → [`AuthenticationError::InvalidToken`].
///
/// Note: callers are responsible for enforcing `MAX_KEY_BYTES` before parsing.
pub struct DefaultApiKeyParser;

impl ApiKeyParser for DefaultApiKeyParser {
    fn parse(&self, raw: &str) -> Result<(ApiKeyId, Secret), AuthenticationError> {
        let (id_part, secret_part) = raw.split_once('.').ok_or_else(|| {
            AuthenticationError::InvalidToken("api key must contain a '.' separator".into())
        })?;
        if id_part.is_empty() {
            return Err(AuthenticationError::InvalidToken(
                "api key id part must not be empty".into(),
            ));
        }
        if secret_part.is_empty() {
            return Err(AuthenticationError::InvalidToken(
                "api key secret part must not be empty".into(),
            ));
        }
        let key_id = ApiKeyId::new(id_part)
            .map_err(|_| AuthenticationError::InvalidToken("invalid key id format".into()))?;
        let secret = Secret::new(secret_part.as_bytes().to_vec());
        Ok((key_id, secret))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parser() -> DefaultApiKeyParser {
        DefaultApiKeyParser
    }

    #[test]
    fn simple_id_dot_secret_parses_correctly() {
        let (id, secret) = parser().parse("id.secret").unwrap();
        assert_eq!(id.as_str(), "id");
        assert_eq!(secret.as_bytes(), b"secret");
    }

    #[test]
    fn dots_after_first_are_kept_in_secret() {
        let (id, secret) = parser().parse("id.sec.ret").unwrap();
        assert_eq!(id.as_str(), "id");
        assert_eq!(secret.as_bytes(), b"sec.ret");
    }

    #[test]
    fn no_dot_returns_invalid_token() {
        assert!(matches!(
            parser().parse("nodothere"),
            Err(AuthenticationError::InvalidToken(_))
        ));
    }

    #[test]
    fn empty_id_half_returns_invalid_token() {
        assert!(matches!(
            parser().parse(".secret"),
            Err(AuthenticationError::InvalidToken(_))
        ));
    }

    #[test]
    fn empty_secret_half_returns_invalid_token() {
        assert!(matches!(
            parser().parse("id."),
            Err(AuthenticationError::InvalidToken(_))
        ));
    }

    #[test]
    fn determinism_same_input_same_output() {
        let raw = "mykey.mysecret";
        let (id1, s1) = parser().parse(raw).unwrap();
        let (id2, s2) = parser().parse(raw).unwrap();
        assert_eq!(id1.as_str(), id2.as_str());
        assert_eq!(s1.as_bytes(), s2.as_bytes());
    }
}
