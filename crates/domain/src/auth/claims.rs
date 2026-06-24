//! JWT claims types — standard registered claims and custom extension claims.
//!
//! Separates the IANA-registered JWT claims ([`StandardClaims`]) from
//! application-specific custom claims ([`Claims`]). All maps use
//! [`BTreeMap`] to guarantee lexicographic key ordering.

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde_json::Value;

/// The IANA-registered "standard" JWT claims.
///
/// All fields are `Option` because registered claims are technically optional
/// in the JWT specification. Providers validate the subset that their
/// configuration requires.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct StandardClaims {
    /// `exp` — token expiration time.
    pub exp: Option<DateTime<Utc>>,

    /// `nbf` — token not-before time (earliest valid time).
    pub nbf: Option<DateTime<Utc>>,

    /// `iat` — token issued-at time.
    pub iat: Option<DateTime<Utc>>,

    /// `jti` — unique JWT identifier.
    pub jti: Option<String>,

    /// `iss` — token issuer.
    pub iss: Option<String>,

    /// `aud` — intended audience(s).
    pub aud: Option<Vec<String>>,
}


/// Combined standard + custom claims extracted from a JWT.
///
/// `custom` holds all claims not mapped to standard fields or identity fields
/// (`sub`, `roles`, `tenant_id`/`tid`). Keys are stored in a [`BTreeMap`] so
/// that iteration order is deterministic and equality checks are stable.
#[derive(Debug, Clone, PartialEq)]
pub struct Claims {
    /// Standard registered JWT claims.
    pub standard: StandardClaims,

    /// All remaining claims, keyed by claim name.
    ///
    /// Uses [`BTreeMap`] for deterministic ordering — never [`std::collections::HashMap`].
    pub custom: BTreeMap<String, Value>,
}

impl Claims {
    /// Creates empty claims (all standard fields `None`, no custom entries).
    pub fn empty() -> Self {
        Self {
            standard: StandardClaims::default(),
            custom: BTreeMap::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use serde_json::json;

    #[test]
    fn standard_claims_default_all_none() {
        let sc = StandardClaims::default();
        assert!(sc.exp.is_none());
        assert!(sc.nbf.is_none());
        assert!(sc.iat.is_none());
        assert!(sc.jti.is_none());
        assert!(sc.iss.is_none());
        assert!(sc.aud.is_none());
    }

    #[test]
    fn claims_empty_has_no_custom() {
        let c = Claims::empty();
        assert!(c.custom.is_empty());
    }

    #[test]
    fn custom_uses_btreemap_ordering() {
        let mut c = Claims::empty();
        c.custom.insert("z_claim".into(), json!("last"));
        c.custom.insert("a_claim".into(), json!("first"));
        let mut keys = c.custom.keys();
        assert_eq!(keys.next().unwrap(), "a_claim");
        assert_eq!(keys.next().unwrap(), "z_claim");
    }

    #[test]
    fn claims_clone_and_eq() {
        let ts = Utc.with_ymd_and_hms(2024, 6, 1, 0, 0, 0).unwrap();
        let c = Claims {
            standard: StandardClaims {
                exp: Some(ts),
                iss: Some("test-iss".into()),
                ..StandardClaims::default()
            },
            custom: {
                let mut m = BTreeMap::new();
                m.insert("foo".into(), json!(42));
                m
            },
        };
        let c2 = c.clone();
        assert_eq!(c, c2);
    }
}
