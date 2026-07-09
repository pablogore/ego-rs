//! `DefaultPrincipalMapper` — maps standard OIDC claims to `(Principal, Claims)`.
//!
//! Also provides `value_to_claim_value` — the boundary conversion from
//! `serde_json::Value` to domain `ClaimValue`. This conversion MUST live here
//! (at the infrastructure boundary), never in `ego-domain` or `security-sdk`
//! (design §AD-OIDC-013).

use std::collections::BTreeMap;

/// Standard JWT registered claim names that are consumed as typed fields.
/// These are stripped from the custom-claims map after extraction.
const STANDARD_JWT_CLAIM_KEYS: &[&str] = &["exp", "nbf", "iat", "jti", "iss", "aud"];

use ego_domain::auth::{ClaimSet, ClaimValue, StandardClaims};
use ego_security_sdk::{Principal, PrincipalKind,     PrincipalMapper, Role, SubjectId};
use ego_domain::auth::AuthenticationError;
use ego_domain::context::TenantId;
use serde_json::Value;

// ---------------------------------------------------------------------------
// serde_json::Value → ClaimValue conversion (pub(crate))
// ---------------------------------------------------------------------------

/// Convert a `serde_json::Value` to a `ClaimValue`.
///
/// Lives at the `security-jwt` infrastructure boundary. Called by
/// `DefaultPrincipalMapper`, `OidcAuthenticationProvider`, and
/// `IntrospectionAuthenticationProvider` when building a `ClaimSet` from
/// a decoded/introspected payload.
pub(crate) fn value_to_claim_value(v: Value) -> ClaimValue {
    match v {
        Value::Null => ClaimValue::Null,
        Value::Bool(b) => ClaimValue::Bool(b),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                ClaimValue::Integer(i)
            } else if let Some(u) = n.as_u64() {
                // u64 that doesn't fit i64 — store as float
                ClaimValue::Float(u as f64)
            } else {
                ClaimValue::Float(n.as_f64().unwrap_or(0.0))
            }
        }
        Value::String(s) => ClaimValue::String(s),
        Value::Array(arr) => {
            ClaimValue::Array(arr.into_iter().map(value_to_claim_value).collect())
        }
        Value::Object(map) => {
            let converted: BTreeMap<String, ClaimValue> = map
                .into_iter()
                .map(|(k, v)| (k, value_to_claim_value(v)))
                .collect();
            ClaimValue::Map(converted)
        }
    }
}

/// Convert a `BTreeMap<String, serde_json::Value>` to a `ClaimSet`.
pub(crate) fn claims_map_to_claim_set(map: BTreeMap<String, Value>) -> ClaimSet {
    let raw: BTreeMap<String, ClaimValue> =
        map.into_iter().map(|(k, v)| (k, value_to_claim_value(v))).collect();
    ClaimSet::new(raw)
}

// ---------------------------------------------------------------------------
// DefaultPrincipalMapper
// ---------------------------------------------------------------------------

/// Maps standard OIDC claims to `(Principal, Claims)`.
///
/// Claim paths read (in priority order per field):
/// - `principal.subject_id` ← `sub`
/// - `principal.roles`      ← `roles` | `realm_access.roles` (Keycloak) | `groups`
/// - `principal.tenant_id`  ← `tenant_id` | `tid` (Entra ID) | `tenant`
/// - `claims["scope"]`      ← `scp` (Azure/Entra ID) | `scope`
/// - `claims["organization"]` ← `organization` | `org_id`
pub struct DefaultPrincipalMapper;

impl PrincipalMapper for DefaultPrincipalMapper {
    fn map(
        &self,
        claim_set: &ClaimSet,
    ) -> Result<(Principal, ego_domain::auth::Claims), AuthenticationError> {
        // sub (required) — distinguish absent (MissingClaim) from wrong-type (InvalidToken)
        let sub = match claim_set.raw.get("sub") {
            None => return Err(AuthenticationError::MissingClaim("sub".into())),
            Some(v) => v.as_str().ok_or_else(|| {
                AuthenticationError::InvalidToken("sub claim is not a string".into())
            })?,
        };

        let mut principal = Principal::new(
            PrincipalKind::User,
            SubjectId::new(sub)
                .map_err(|_| AuthenticationError::InvalidToken("invalid subject id".into()))?,
        );

        // roles (first present wins); track which key(s) were consumed
        let roles_vec = claim_set.roles();
        let roles_consumed = !roles_vec.is_empty();
        for role in &roles_vec {
            principal = principal.with_role(Role(role.to_string()));
        }
        // Determine which role key was the source (to remove from custom)
        let roles_source_key: Option<&str> = if roles_consumed {
            if claim_set.get_array("roles").is_some() {
                Some("roles")
            } else if claim_set.get_nested_array("realm_access", "roles").is_some() {
                Some("realm_access")
            } else {
                Some("groups")
            }
        } else {
            None
        };

        // tenant — track which key was consumed
        let (tenant_id, tenant_key_consumed): (Option<String>, Option<&str>) =
            if let Some(tid) = claim_set.get_str("tenant_id") {
                (Some(tid.to_string()), Some("tenant_id"))
            } else if let Some(tid) = claim_set.get_str("tid") {
                (Some(tid.to_string()), Some("tid"))
            } else if let Some(t) = claim_set.get_str("tenant") {
                (Some(t.to_string()), Some("tenant"))
            } else {
                (None, None)
            };

        if let Some(tid) = tenant_id {
            let tenant = TenantId::new(tid)
                .map_err(|_| AuthenticationError::InvalidToken("invalid tenant claim".into()))?;
            principal = principal.with_tenant_id(tenant);
        }

        // Build standard claims from the claim set
        let standard = build_standard_from_claim_set(claim_set);

        // Build custom claims — start with ALL claims, then selectively remove consumed ones
        let mut custom: BTreeMap<String, Value> = claim_set
            .raw
            .iter()
            .map(|(k, v)| (k.clone(), claim_value_to_json(v)))
            .collect();

        // Always remove: sub (principal), standard JWT claims
        custom.remove("sub");
        for key in STANDARD_JWT_CLAIM_KEYS {
            custom.remove(*key);
        }

        // Remove role-related key only if it was consumed (i.e., was the right type)
        if let Some(key) = roles_source_key {
            custom.remove(key);
        }
        // Remove tenant key only if it was consumed as a string
        if let Some(key) = tenant_key_consumed {
            custom.remove(key);
        }

        // Promote scope
        if let Some(scope) = claim_set.scope() {
            custom.insert("scope".to_string(), Value::String(scope.to_string()));
            custom.remove("scp");
        }

        // Promote organization
        if let Some(org) = claim_set.organization() {
            custom.insert("organization".to_string(), Value::String(org.to_string()));
            custom.remove("org_id");
        }

        let claims = ego_domain::auth::Claims { standard, custom };
        Ok((principal, claims))
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

fn build_standard_from_claim_set(cs: &ClaimSet) -> StandardClaims {
    let exp = cs.get_i64("exp")
        .and_then(|ts| chrono::DateTime::from_timestamp(ts, 0));
    let nbf = cs.get_i64("nbf")
        .and_then(|ts| chrono::DateTime::from_timestamp(ts, 0));
    let iat = cs.get_i64("iat")
        .and_then(|ts| chrono::DateTime::from_timestamp(ts, 0));
    let jti = cs.get_str("jti").map(str::to_owned);
    let iss = cs.get_str("iss").map(str::to_owned);
    let aud = cs.get_array("aud").map(|arr| {
        arr.iter().filter_map(ClaimValue::as_str).map(str::to_owned).collect()
    }).or_else(|| cs.get_str("aud").map(|s| vec![s.to_owned()]));

    StandardClaims { exp, nbf, iat, jti, iss, aud }
}

/// Convert a `ClaimValue` back to `serde_json::Value` for storage in `Claims.custom`.
pub(crate) fn claim_value_to_json(v: &ClaimValue) -> Value {
    match v {
        ClaimValue::Null => Value::Null,
        ClaimValue::Bool(b) => Value::Bool(*b),
        ClaimValue::Integer(i) => Value::Number((*i).into()),
        ClaimValue::Float(f) => serde_json::Number::from_f64(*f)
            .map(Value::Number)
            .unwrap_or(Value::Null),
        ClaimValue::String(s) => Value::String(s.clone()),
        ClaimValue::Array(arr) => {
            Value::Array(arr.iter().map(claim_value_to_json).collect())
        }
        ClaimValue::Map(map) => {
            let obj: serde_json::Map<String, Value> = map
                .iter()
                .map(|(k, v)| (k.clone(), claim_value_to_json(v)))
                .collect();
            Value::Object(obj)
        }
        // #[non_exhaustive] wildcard — future ClaimValue variants serialize as null
        _ => Value::Null,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use ego_domain::auth::{ClaimSet, ClaimValue};

    fn make_claims(pairs: Vec<(&str, ClaimValue)>) -> ClaimSet {
        let mut raw = BTreeMap::new();
        for (k, v) in pairs {
            raw.insert(k.to_string(), v);
        }
        ClaimSet::new(raw)
    }

    // --- value_to_claim_value ---

    #[test]
    fn null_converts_to_null() {
        assert_eq!(value_to_claim_value(Value::Null), ClaimValue::Null);
    }

    #[test]
    fn bool_converts() {
        assert_eq!(value_to_claim_value(Value::Bool(true)), ClaimValue::Bool(true));
    }

    #[test]
    fn integer_converts() {
        let v = serde_json::json!(42i64);
        assert_eq!(value_to_claim_value(v), ClaimValue::Integer(42));
    }

    #[test]
    fn float_converts() {
        let v = serde_json::json!(2.5f64);
        assert!(matches!(value_to_claim_value(v), ClaimValue::Float(_)));
    }

    #[test]
    fn string_converts() {
        let v = serde_json::json!("hello");
        assert_eq!(value_to_claim_value(v), ClaimValue::String("hello".into()));
    }

    #[test]
    fn array_converts_recursively() {
        let v = serde_json::json!(["a", "b"]);
        match value_to_claim_value(v) {
            ClaimValue::Array(arr) => {
                assert_eq!(arr.len(), 2);
                assert_eq!(arr[0].as_str(), Some("a"));
            }
            _ => panic!("expected Array"),
        }
    }

    #[test]
    fn object_converts_to_map() {
        let v = serde_json::json!({"key": "val"});
        match value_to_claim_value(v) {
            ClaimValue::Map(m) => {
                assert_eq!(m.get("key").and_then(ClaimValue::as_str), Some("val"));
            }
            _ => panic!("expected Map"),
        }
    }

    // --- DefaultPrincipalMapper ---

    #[test]
    fn maps_sub_to_subject_id() {
        let cs = make_claims(vec![
            ("sub", ClaimValue::String("user-1".into())),
            ("exp", ClaimValue::Integer(9_999_999_999)),
        ]);
        let (principal, _) = DefaultPrincipalMapper.map(&cs).unwrap();
        assert_eq!(principal.subject_id.as_str(), "user-1");
    }

    #[test]
    fn maps_roles_to_principal_roles() {
        let cs = make_claims(vec![
            ("sub", ClaimValue::String("u1".into())),
            ("roles", ClaimValue::Array(vec![ClaimValue::String("admin".into())])),
            ("exp", ClaimValue::Integer(9_999_999_999)),
        ]);
        let (principal, _) = DefaultPrincipalMapper.map(&cs).unwrap();
        assert!(principal.roles.iter().any(|r| r.0 == "admin"));
    }

    #[test]
    fn maps_realm_access_roles() {
        let mut realm = BTreeMap::new();
        realm.insert(
            "roles".to_string(),
            ClaimValue::Array(vec![ClaimValue::String("editor".into())]),
        );
        let cs = make_claims(vec![
            ("sub", ClaimValue::String("u1".into())),
            ("realm_access", ClaimValue::Map(realm)),
            ("exp", ClaimValue::Integer(9_999_999_999)),
        ]);
        let (principal, _) = DefaultPrincipalMapper.map(&cs).unwrap();
        assert!(principal.roles.iter().any(|r| r.0 == "editor"));
    }

    #[test]
    fn maps_groups_to_principal_roles() {
        let cs = make_claims(vec![
            ("sub", ClaimValue::String("u1".into())),
            ("groups", ClaimValue::Array(vec![ClaimValue::String("viewers".into())])),
            ("exp", ClaimValue::Integer(9_999_999_999)),
        ]);
        let (principal, _) = DefaultPrincipalMapper.map(&cs).unwrap();
        assert!(principal.roles.iter().any(|r| r.0 == "viewers"));
    }

    #[test]
    fn maps_tid_to_tenant_id() {
        let cs = make_claims(vec![
            ("sub", ClaimValue::String("u1".into())),
            ("tid", ClaimValue::String("tenant-42".into())),
            ("exp", ClaimValue::Integer(9_999_999_999)),
        ]);
        let (principal, _) = DefaultPrincipalMapper.map(&cs).unwrap();
        assert_eq!(
            principal.tenant_id.as_ref().map(TenantId::as_str),
            Some("tenant-42")
        );
    }

    #[test]
    fn maps_invalid_tenant_claim_fails() {
        let cs = make_claims(vec![
            ("sub", ClaimValue::String("u1".into())),
            ("tid", ClaimValue::String("   ".into())),
            ("exp", ClaimValue::Integer(9_999_999_999)),
        ]);
        let err = DefaultPrincipalMapper.map(&cs).unwrap_err();
        assert!(matches!(err, AuthenticationError::InvalidToken(_)));
    }

    #[test]
    fn maps_scp_to_scope_in_custom() {
        let cs = make_claims(vec![
            ("sub", ClaimValue::String("u1".into())),
            ("scp", ClaimValue::String("read write".into())),
            ("exp", ClaimValue::Integer(9_999_999_999)),
        ]);
        let (_, claims) = DefaultPrincipalMapper.map(&cs).unwrap();
        let scope = claims.custom.get("scope").and_then(Value::as_str);
        assert_eq!(scope, Some("read write"));
    }

    #[test]
    fn maps_organization_to_custom() {
        let cs = make_claims(vec![
            ("sub", ClaimValue::String("u1".into())),
            ("organization", ClaimValue::String("acme".into())),
            ("exp", ClaimValue::Integer(9_999_999_999)),
        ]);
        let (_, claims) = DefaultPrincipalMapper.map(&cs).unwrap();
        let org = claims.custom.get("organization").and_then(Value::as_str);
        assert_eq!(org, Some("acme"));
    }

    #[test]
    fn missing_sub_returns_missing_claim() {
        let cs = make_claims(vec![("exp", ClaimValue::Integer(9_999_999_999))]);
        let err = DefaultPrincipalMapper.map(&cs).unwrap_err();
        assert!(matches!(err, AuthenticationError::MissingClaim(ref s) if s == "sub"));
    }
}
