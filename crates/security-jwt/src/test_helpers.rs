use std::sync::Arc;

use chrono::{DateTime, Utc};
use ego_domain::auth::Clock;
use ego_testkit::TestJwtBuilder;

pub(crate) struct FixedClock(pub(crate) chrono::DateTime<Utc>);

impl Clock for FixedClock {
    fn now(&self) -> chrono::DateTime<Utc> {
        self.0
    }
}

pub(crate) fn fixed_clock(ts: chrono::DateTime<Utc>) -> Arc<dyn Clock> {
    Arc::new(FixedClock(ts))
}

/// A fixed anchor in the past that is always valid for testing.
/// Using a hardcoded timestamp ensures 100% deterministic tests (W-09).
#[cfg(test)]
pub(crate) fn pinned_now() -> DateTime<Utc> {
    DateTime::from_timestamp(1_750_000_000, 0).expect("valid timestamp")
}

pub(crate) fn now_clock() -> Arc<dyn Clock> {
    fixed_clock(pinned_now())
}

pub(crate) fn hs256_secret() -> Vec<u8> {
    b"super-secret-key-for-testing-only".to_vec()
}

/// Audience minted into test tokens by [`make_hs256_token`] and matched by the
/// provider tests' `expected_aud`. `expected_aud` is a required config field
/// (audience-confusion / token-reuse defense), so provider construction and
/// token validation both need a concrete, matching audience.
pub(crate) const TEST_AUD: &str = "test-audience";

/// Builds an HS256 token from a caller-supplied claims object, via
/// `TestJwtBuilder`'s `claim()` escape hatch (each field of `claims` is set
/// individually, overwriting `TestJwtBuilder`'s default `exp`). Callers that
/// need a token with NO `exp` claim at all cannot use this helper (see
/// `TestJwtBuilder`'s design note) and build their own token directly.
///
/// When the caller does not specify an `aud` claim, a default [`TEST_AUD`]
/// audience is added so tokens authenticate against providers whose
/// (now required) `expected_aud` is [`TEST_AUD`]. Callers that assert on a
/// specific `aud` value simply set it themselves and keep control.
pub(crate) fn make_hs256_token(claims: &serde_json::Value) -> String {
    let mut builder = TestJwtBuilder::new(hs256_secret());
    let has_aud = claims
        .as_object()
        .is_some_and(|obj| obj.contains_key("aud"));
    if !has_aud {
        builder = builder.claim("aud", serde_json::Value::from(TEST_AUD));
    }
    if let Some(obj) = claims.as_object() {
        for (key, value) in obj {
            builder = builder.claim(key, value.clone());
        }
    }
    builder.build()
}

pub(crate) fn future_ts(offset_secs: i64) -> i64 {
    (pinned_now() + chrono::Duration::seconds(offset_secs)).timestamp()
}

pub(crate) fn past_ts(offset_secs: i64) -> i64 {
    (pinned_now() - chrono::Duration::seconds(offset_secs)).timestamp()
}
