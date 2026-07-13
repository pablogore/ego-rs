//! Reusable HS256 test-token builder (CORE-026, design.md AD-2).
//!
//! Replaces three independently hand-rolled HS256 JWT-minting
//! implementations found across the workspace. A direct `jsonwebtoken`
//! dependency is used here (not `security-jwt`'s `FakeIssuer`) to keep
//! `ego-testkit` free of a `testkit -> security-jwt` edge, since
//! `security-jwt` itself dev-depends on `ego-testkit`.

use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
use serde_json::{Map, Value};
use std::collections::HashSet;
use std::time::{SystemTime, UNIX_EPOCH};

/// Builds a signed HS256 test JWT with caller-specified claims.
///
/// A default `exp` claim (now + 3600s) is seeded at construction so a test
/// author who does not customize anything still gets an immediately usable
/// token. Every setter overwrites its corresponding claim; `claim()` is the
/// escape hatch for anything not covered by a dedicated setter.
pub struct TestJwtBuilder {
    signing_key: Vec<u8>,
    claims: Map<String, Value>,
    named_claims: HashSet<&'static str>,
}

impl TestJwtBuilder {
    /// Starts a new builder for the given HS256 signing key, with a default
    /// `exp` claim of now + 3600 seconds.
    pub fn new(signing_key: impl Into<Vec<u8>>) -> Self {
        let mut claims = Map::new();
        claims.insert("exp".to_string(), Value::from(default_exp()));
        Self { signing_key: signing_key.into(), claims, named_claims: HashSet::new() }
    }

    /// Sets the `sub` claim. Omit to build a token with no `sub` claim
    /// (serves negative tests).
    pub fn subject(mut self, sub: &str) -> Self {
        self.claims.insert("sub".to_string(), Value::from(sub));
        self.named_claims.insert("sub");
        self
    }

    /// Sets the `tenant_id` claim.
    pub fn tenant_id(mut self, tenant_id: &str) -> Self {
        self.claims.insert("tenant_id".to_string(), Value::from(tenant_id));
        self.named_claims.insert("tenant_id");
        self
    }

    /// Overrides the default `exp` claim with an explicit unix timestamp
    /// (past/future-exp negative tests).
    pub fn expires_at(mut self, unix_ts: i64) -> Self {
        self.claims.insert("exp".to_string(), Value::from(unix_ts));
        self
    }

    /// Sets an arbitrary claim by name — the escape hatch for anything not
    /// covered by a dedicated setter. Panics if `key` was already set through
    /// its dedicated named method (`.subject()`/`.tenant_id()`), so a
    /// reserved claim can't be silently overwritten by the escape hatch.
    pub fn claim(mut self, key: &str, value: Value) -> Self {
        assert!(
            !self.named_claims.contains(key),
            "TestJwtBuilder::claim(\"{key}\") collides with a value already set via \
             its dedicated named method — use that method instead of the escape hatch"
        );
        self.claims.insert(key.to_string(), value);
        self
    }

    /// Encodes and signs the accumulated claims as an HS256 JWT.
    pub fn build(self) -> String {
        encode(
            &Header::new(Algorithm::HS256),
            &Value::Object(self.claims),
            &EncodingKey::from_secret(&self.signing_key),
        )
        .expect("TestJwtBuilder: HS256 encode over well-formed claims never fails")
    }
}

fn default_exp() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock is after UNIX_EPOCH")
        .as_secs() as i64
        + 3600
}

#[cfg(test)]
mod tests {
    use super::TestJwtBuilder;
    use jsonwebtoken::{decode, Algorithm, DecodingKey, Validation};
    use serde_json::Value;

    fn secret() -> Vec<u8> {
        b"testkit-jwt-builder-secret-32-bytes".to_vec()
    }

    fn decode_claims(token: &str) -> Value {
        decode::<Value>(token, &DecodingKey::from_secret(&secret()), &Validation::new(Algorithm::HS256))
            .expect("token built by TestJwtBuilder must verify as a valid HS256 token")
            .claims
    }

    #[test]
    fn build_produces_a_token_that_verifies_with_matching_claims() {
        let token = TestJwtBuilder::new(secret())
            .subject("user-1")
            .tenant_id("tenant-a")
            .build();

        let claims = decode_claims(&token);

        assert_eq!(claims["sub"], Value::from("user-1"));
        assert_eq!(claims["tenant_id"], Value::from("tenant-a"));
        assert!(claims["exp"].is_i64(), "default exp claim must be present");
    }

    #[test]
    fn omitting_subject_yields_no_sub_claim() {
        let token = TestJwtBuilder::new(secret()).tenant_id("tenant-a").build();

        let claims = decode_claims(&token);

        assert!(!claims.as_object().unwrap().contains_key("sub"));
    }

    #[test]
    fn expires_at_overrides_the_default_exp_claim() {
        let token = TestJwtBuilder::new(secret()).expires_at(123).build();

        let mut validation = Validation::new(Algorithm::HS256);
        validation.validate_exp = false; // 123 is far in the past — decode raw claims only
        let claims = decode::<Value>(&token, &DecodingKey::from_secret(&secret()), &validation)
            .expect("token must still verify (signature-only check here)")
            .claims;

        assert_eq!(claims["exp"], Value::from(123));
    }

    #[test]
    fn claim_sets_an_arbitrary_field() {
        let token = TestJwtBuilder::new(secret())
            .claim("scope", Value::from("read write"))
            .build();

        let claims = decode_claims(&token);

        assert_eq!(claims["scope"], Value::from("read write"));
    }

    #[test]
    #[should_panic(expected = "collides")]
    fn claim_rejects_a_reserved_key_collision() {
        TestJwtBuilder::new(secret())
            .tenant_id("tenant-a")
            .claim("tenant_id", Value::from("tenant-b"))
            .build();
    }

    #[test]
    fn claim_sets_a_reserved_key_when_no_named_method_was_called() {
        // No collision: `sub` was never set via `.subject()`, so a raw
        // claims-object builder (e.g. one needing a non-string `sub` for a
        // negative test) may still set it through the escape hatch.
        let token = TestJwtBuilder::new(secret()).claim("sub", Value::from(42)).build();

        let claims = decode_claims(&token);

        assert_eq!(claims["sub"], Value::from(42));
    }
}
