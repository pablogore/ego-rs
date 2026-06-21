//! Credential types presented before authentication.

/// What a caller presents before authentication.
///
/// Credentials are inputs to authentication only — they are never stored on
/// a [`crate::principal::Principal`]. Holds no transport types (no HTTP headers,
/// no gRPC metadata).
#[derive(Debug, Clone)]
pub enum Credential {
    /// Username + secret (Basic scheme).
    Basic {
        /// The username.
        username: String,
        /// The shared secret / password.
        secret: String,
    },
    /// A bearer token (e.g. a JWT) as an opaque string.
    Bearer(String),
    /// Any other scheme, with a free-form raw-bytes payload.
    Custom {
        /// Scheme name (e.g. `"api-key"`).
        scheme: String,
        /// Opaque raw bytes payload for that scheme.
        payload: Vec<u8>,
    },
}

#[cfg(test)]
mod tests {
    use super::Credential;

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
    fn bearer_variant_constructs_and_matches() {
        let cred = Credential::Bearer("tok.en.here".into());
        match cred {
            Credential::Bearer(token) => assert_eq!(token, "tok.en.here"),
            _ => panic!("expected Bearer variant"),
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

    #[test]
    fn custom_payload_is_raw_bytes() {
        // Verify payload accepts arbitrary byte sequences (not just UTF-8 strings).
        let cred = Credential::Custom {
            scheme: "binary".into(),
            payload: vec![0u8, 1, 2],
        };
        match cred {
            Credential::Custom { payload, .. } => assert_eq!(payload.len(), 3),
            _ => panic!("expected Custom variant"),
        }
    }

    #[test]
    fn credential_is_no_transport_type() {
        // Compile-time check: Credential must be Clone + Debug (no transport types prevent these).
        fn assert_clone_debug<T: Clone + std::fmt::Debug>() {}
        assert_clone_debug::<Credential>();
    }
}
