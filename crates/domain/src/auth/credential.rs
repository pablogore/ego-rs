//! Authentication credentials presented to an authentication provider.
//!
//! Callers wrap the raw credential material in a [`Credential`] variant
//! before calling `authenticate`. The provider is responsible for extracting
//! and validating the material.

use std::fmt;

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
#[derive(Clone, PartialEq, Eq)]
pub enum Credential {
    /// Username + secret (Basic scheme).
    Basic {
        /// The username.
        username: String,
        /// The shared secret / password.
        secret: String,
    },
    /// A bearer token (e.g. a JWT) as an opaque string (without the `"Bearer "` prefix).
    Bearer(String),
    /// Any other scheme, with a free-form raw-bytes payload.
    Custom {
        /// Scheme name (e.g. `"api-key"`).
        scheme: String,
        /// Opaque raw bytes payload for that scheme.
        payload: Vec<u8>,
    },
}

// Intentionally exhaustive — no wildcard arm and no `..` field remainder:
// adding a new variant OR a new field to an existing variant is a compile
// error, forcing the author to decide how to present (and whether to redact)
// the new credential material.
impl fmt::Debug for Credential {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Basic {
                username,
                secret: _,
            } => f
                .debug_struct("Basic")
                .field("username", username)
                .field("secret", &"[REDACTED]")
                .finish(),
            Self::Bearer(_) => f.debug_tuple("Bearer").field(&"[REDACTED]").finish(),
            Self::Custom { scheme, payload } => f
                .debug_struct("Custom")
                .field("scheme", scheme)
                .field("payload", &format_args!("[{} bytes]", payload.len()))
                .finish(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bearer_stores_value() {
        let c = Credential::Bearer("tok".into());
        assert!(matches!(c, Credential::Bearer(ref s) if s == "tok"));
    }

    #[test]
    fn credential_is_clone_and_eq() {
        let a = Credential::Bearer("abc".into());
        let b = a.clone();
        assert_eq!(a, b);
    }

    #[test]
    fn bearer_debug_redacts_token() {
        let c = Credential::Bearer("eyJhbGciOiJSUzI1NiJ9.secret".into());
        let s = format!("{c:?}");
        assert!(s.contains("Bearer"), "variant name must appear");
        assert!(
            !s.contains("eyJhbGciOiJSUzI1NiJ9"),
            "Bearer token must not appear in debug output"
        );
        assert!(s.contains("[REDACTED]"));
    }

    #[test]
    fn basic_debug_redacts_secret() {
        let c = Credential::Basic {
            username: "alice".into(),
            secret: "hunter2".into(),
        };
        let s = format!("{c:?}");
        assert!(s.contains("alice"), "username must appear");
        assert!(
            !s.contains("hunter2"),
            "secret must not appear in debug output"
        );
        assert!(s.contains("[REDACTED]"));
    }

    #[test]
    fn custom_debug_hides_payload() {
        let c = Credential::Custom {
            scheme: "api-key".into(),
            payload: b"\xff\xfe\x00".to_vec(),
        };
        let s = format!("{c:?}");
        assert!(s.contains("api-key"), "scheme must appear");
        assert!(
            s.contains("[3 bytes]"),
            "payload must be shown as byte count only"
        );
    }

    #[test]
    fn basic_variant_constructs_and_matches() {
        let cred = Credential::Basic {
            username: "bob".into(),
            secret: "pw".into(),
        };
        match cred {
            Credential::Basic { username, secret } => {
                assert_eq!(username, "bob");
                assert_eq!(secret, "pw");
            }
            _ => panic!("expected Basic variant"),
        }
    }

    #[test]
    fn custom_variant_constructs_and_matches() {
        let cred = Credential::Custom {
            scheme: "X-Api-Key".into(),
            payload: b"key123".to_vec(),
        };
        match cred {
            Credential::Custom { scheme, payload } => {
                assert_eq!(scheme, "X-Api-Key");
                assert_eq!(payload, b"key123");
            }
            _ => panic!("expected Custom variant"),
        }
    }
}
