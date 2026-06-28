use std::sync::Arc;

use chrono::{DateTime, Utc};
use ego_domain::auth::Clock;
use jsonwebtoken::{encode, EncodingKey, Header};

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

pub(crate) fn make_hs256_token(claims: &serde_json::Value) -> String {
    encode(
        &Header::new(jsonwebtoken::Algorithm::HS256),
        claims,
        &EncodingKey::from_secret(&hs256_secret()),
    )
    .unwrap()
}

pub(crate) fn future_ts(offset_secs: i64) -> i64 {
    (pinned_now() + chrono::Duration::seconds(offset_secs)).timestamp()
}

pub(crate) fn past_ts(offset_secs: i64) -> i64 {
    (pinned_now() - chrono::Duration::seconds(offset_secs)).timestamp()
}
