//! ClaimSet and ClaimValue domain value objects.
//!
//! Pure domain types — no `serde_json` dependency. The `serde_json::Value →
//! ClaimValue` conversion lives in `security-jwt` at the infrastructure
//! boundary (design §AD-OIDC-013).

use std::collections::BTreeMap;

/// Raw claim value. Mirrors JSON's type lattice but is an owned domain type.
///
/// `#[non_exhaustive]` — future variants (e.g. binary data) can be added
/// without breaking existing `match` arms.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq)]
pub enum ClaimValue {
    /// A UTF-8 string.
    String(String),
    /// A 64-bit signed integer.
    Integer(i64),
    /// A 64-bit float.
    Float(f64),
    /// A boolean.
    Bool(bool),
    /// An ordered array of claim values.
    Array(Vec<ClaimValue>),
    /// A nested map (e.g. `realm_access.roles` in Keycloak tokens).
    Map(BTreeMap<String, ClaimValue>),
    /// JSON null.
    Null,
}

impl ClaimValue {
    /// Returns the inner `&str` if this is a `String` variant.
    pub fn as_str(&self) -> Option<&str> {
        match self {
            ClaimValue::String(s) => Some(s.as_str()),
            _ => None,
        }
    }
}

/// Carries the raw claims from a verified token as a domain value object.
///
/// Decouples `security-sdk` and callers from `serde_json::Value`.
#[derive(Debug, Clone, PartialEq)]
pub struct ClaimSet {
    /// Raw claims map — all fields from the verified token payload.
    pub raw: BTreeMap<String, ClaimValue>,
}

impl ClaimSet {
    /// Creates an empty claim set.
    pub fn new(raw: BTreeMap<String, ClaimValue>) -> Self {
        Self { raw }
    }

    // --- Raw access (base layer) ---

    /// Returns the string value for `key`, or `None` if absent or not a string.
    pub fn get_str(&self, key: &str) -> Option<&str> {
        self.raw.get(key).and_then(ClaimValue::as_str)
    }

    /// Returns the array value for `key`, or `None` if absent or not an array.
    pub fn get_array(&self, key: &str) -> Option<&[ClaimValue]> {
        match self.raw.get(key) {
            Some(ClaimValue::Array(arr)) => Some(arr.as_slice()),
            _ => None,
        }
    }

    /// Returns an array nested one level deep: `outer.inner`
    /// (e.g. `realm_access.roles` in Keycloak tokens).
    pub fn get_nested_array(&self, outer: &str, inner: &str) -> Option<&[ClaimValue]> {
        match self.raw.get(outer) {
            Some(ClaimValue::Map(m)) => match m.get(inner) {
                Some(ClaimValue::Array(arr)) => Some(arr.as_slice()),
                _ => None,
            },
            _ => None,
        }
    }

    /// Returns a numeric claim as `i64`, truncating any fractional part.
    pub fn get_i64(&self, key: &str) -> Option<i64> {
        match self.raw.get(key) {
            Some(ClaimValue::Integer(n)) => Some(*n),
            Some(ClaimValue::Float(f)) => Some(*f as i64),
            _ => None,
        }
    }

    // --- Standard identity helpers ---

    /// Subject identifier (`sub` claim). Always present in valid OIDC tokens.
    pub fn subject(&self) -> Option<&str> {
        self.get_str("sub")
    }

    /// Roles from `roles`, `realm_access.roles` (Keycloak-style nested),
    /// or `groups` — first present and non-empty wins.
    pub fn roles(&self) -> Vec<&str> {
        let arr = self
            .get_array("roles")
            .or_else(|| self.get_nested_array("realm_access", "roles"))
            .or_else(|| self.get_array("groups"));
        arr.map(|v| v.iter().filter_map(ClaimValue::as_str).collect())
            .unwrap_or_default()
    }

    /// Tenant identifier from `tenant_id`, `tid` (Entra ID), or `tenant`.
    pub fn tenant(&self) -> Option<&str> {
        self.get_str("tenant_id")
            .or_else(|| self.get_str("tid"))
            .or_else(|| self.get_str("tenant"))
    }

    /// OAuth2 scopes from `scp` (Azure/Entra ID) or `scope`.
    pub fn scope(&self) -> Option<&str> {
        self.get_str("scp").or_else(|| self.get_str("scope"))
    }

    /// Organization identifier from `organization` or `org_id`.
    pub fn organization(&self) -> Option<&str> {
        self.get_str("organization")
            .or_else(|| self.get_str("org_id"))
    }

    /// Token expiry timestamp (`exp` claim).
    pub fn expiry(&self) -> Option<i64> {
        self.get_i64("exp")
    }

    /// Token issuer (`iss` claim).
    pub fn issuer(&self) -> Option<&str> {
        self.get_str("iss")
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn make_claims(pairs: Vec<(&str, ClaimValue)>) -> ClaimSet {
        let mut raw = BTreeMap::new();
        for (k, v) in pairs {
            raw.insert(k.to_string(), v);
        }
        ClaimSet::new(raw)
    }

    // --- ClaimValue helpers ---

    #[test]
    fn claim_value_as_str_returns_some_for_string_variant() {
        let v = ClaimValue::String("hello".into());
        assert_eq!(v.as_str(), Some("hello"));
    }

    #[test]
    fn claim_value_as_str_returns_none_for_integer() {
        assert_eq!(ClaimValue::Integer(42).as_str(), None);
    }

    #[test]
    fn claim_value_as_str_returns_none_for_bool() {
        assert_eq!(ClaimValue::Bool(true).as_str(), None);
    }

    #[test]
    fn claim_value_as_str_returns_none_for_null() {
        assert_eq!(ClaimValue::Null.as_str(), None);
    }

    // --- get_str ---

    #[test]
    fn get_str_returns_value_for_string_claim() {
        let cs = make_claims(vec![("sub", ClaimValue::String("user-1".into()))]);
        assert_eq!(cs.get_str("sub"), Some("user-1"));
    }

    #[test]
    fn get_str_returns_none_for_absent_key() {
        let cs = make_claims(vec![]);
        assert_eq!(cs.get_str("sub"), None);
    }

    #[test]
    fn get_str_returns_none_for_non_string_value() {
        let cs = make_claims(vec![("exp", ClaimValue::Integer(1234))]);
        assert_eq!(cs.get_str("exp"), None);
    }

    // --- get_array ---

    #[test]
    fn get_array_returns_slice_for_array_claim() {
        let cs = make_claims(vec![(
            "roles",
            ClaimValue::Array(vec![ClaimValue::String("admin".into())]),
        )]);
        let arr = cs.get_array("roles").unwrap();
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0].as_str(), Some("admin"));
    }

    #[test]
    fn get_array_returns_none_for_absent_key() {
        let cs = make_claims(vec![]);
        assert!(cs.get_array("roles").is_none());
    }

    // --- get_nested_array ---

    #[test]
    fn get_nested_array_returns_keycloak_roles() {
        let mut realm = BTreeMap::new();
        realm.insert(
            "roles".to_string(),
            ClaimValue::Array(vec![ClaimValue::String("editor".into())]),
        );
        let cs = make_claims(vec![("realm_access", ClaimValue::Map(realm))]);
        let arr = cs.get_nested_array("realm_access", "roles").unwrap();
        assert_eq!(arr[0].as_str(), Some("editor"));
    }

    #[test]
    fn get_nested_array_returns_none_if_outer_absent() {
        let cs = make_claims(vec![]);
        assert!(cs.get_nested_array("realm_access", "roles").is_none());
    }

    #[test]
    fn get_nested_array_returns_none_if_inner_absent() {
        let mut realm = BTreeMap::new();
        realm.insert("other".to_string(), ClaimValue::Null);
        let cs = make_claims(vec![("realm_access", ClaimValue::Map(realm))]);
        assert!(cs.get_nested_array("realm_access", "roles").is_none());
    }

    #[test]
    fn get_nested_array_returns_none_if_outer_not_a_map() {
        let cs = make_claims(vec![("realm_access", ClaimValue::String("flat".into()))]);
        assert!(cs.get_nested_array("realm_access", "roles").is_none());
    }

    // --- get_i64 ---

    #[test]
    fn get_i64_returns_integer_claim() {
        let cs = make_claims(vec![("exp", ClaimValue::Integer(9999))]);
        assert_eq!(cs.get_i64("exp"), Some(9999));
    }

    #[test]
    fn get_i64_truncates_float() {
        let cs = make_claims(vec![("exp", ClaimValue::Float(1234.9))]);
        assert_eq!(cs.get_i64("exp"), Some(1234));
    }

    #[test]
    fn get_i64_returns_none_for_string() {
        let cs = make_claims(vec![("exp", ClaimValue::String("never".into()))]);
        assert_eq!(cs.get_i64("exp"), None);
    }

    // --- roles() ---

    #[test]
    fn roles_returns_direct_roles_claim() {
        let cs = make_claims(vec![(
            "roles",
            ClaimValue::Array(vec![
                ClaimValue::String("admin".into()),
                ClaimValue::String("viewer".into()),
            ]),
        )]);
        let roles = cs.roles();
        assert!(roles.contains(&"admin"));
        assert!(roles.contains(&"viewer"));
    }

    #[test]
    fn roles_falls_back_to_realm_access_roles() {
        let mut realm = BTreeMap::new();
        realm.insert(
            "roles".to_string(),
            ClaimValue::Array(vec![ClaimValue::String("editor".into())]),
        );
        let cs = make_claims(vec![("realm_access", ClaimValue::Map(realm))]);
        let roles = cs.roles();
        assert_eq!(roles, vec!["editor"]);
    }

    #[test]
    fn roles_falls_back_to_groups() {
        let cs = make_claims(vec![(
            "groups",
            ClaimValue::Array(vec![ClaimValue::String("viewers".into())]),
        )]);
        assert_eq!(cs.roles(), vec!["viewers"]);
    }

    #[test]
    fn roles_returns_empty_when_none_present() {
        let cs = make_claims(vec![]);
        assert!(cs.roles().is_empty());
    }

    // --- tenant() ---

    #[test]
    fn tenant_returns_tenant_id_first() {
        let cs = make_claims(vec![
            ("tenant_id", ClaimValue::String("primary".into())),
            ("tid", ClaimValue::String("secondary".into())),
        ]);
        assert_eq!(cs.tenant(), Some("primary"));
    }

    #[test]
    fn tenant_falls_back_to_tid() {
        let cs = make_claims(vec![("tid", ClaimValue::String("tenant-42".into()))]);
        assert_eq!(cs.tenant(), Some("tenant-42"));
    }

    #[test]
    fn tenant_falls_back_to_tenant() {
        let cs = make_claims(vec![("tenant", ClaimValue::String("acme".into()))]);
        assert_eq!(cs.tenant(), Some("acme"));
    }

    #[test]
    fn tenant_returns_none_when_absent() {
        let cs = make_claims(vec![]);
        assert!(cs.tenant().is_none());
    }

    // --- scope() ---

    #[test]
    fn scope_returns_scp_first() {
        let cs = make_claims(vec![
            ("scp", ClaimValue::String("read write".into())),
            ("scope", ClaimValue::String("other".into())),
        ]);
        assert_eq!(cs.scope(), Some("read write"));
    }

    #[test]
    fn scope_falls_back_to_scope() {
        let cs = make_claims(vec![("scope", ClaimValue::String("openid profile".into()))]);
        assert_eq!(cs.scope(), Some("openid profile"));
    }

    // --- organization() ---

    #[test]
    fn organization_returns_organization_first() {
        let cs = make_claims(vec![("organization", ClaimValue::String("acme".into()))]);
        assert_eq!(cs.organization(), Some("acme"));
    }

    #[test]
    fn organization_falls_back_to_org_id() {
        let cs = make_claims(vec![("org_id", ClaimValue::String("org-123".into()))]);
        assert_eq!(cs.organization(), Some("org-123"));
    }

    // --- standard helpers ---

    #[test]
    fn subject_returns_sub_claim() {
        let cs = make_claims(vec![("sub", ClaimValue::String("user-1".into()))]);
        assert_eq!(cs.subject(), Some("user-1"));
    }

    #[test]
    fn expiry_returns_exp_claim() {
        let cs = make_claims(vec![("exp", ClaimValue::Integer(9000))]);
        assert_eq!(cs.expiry(), Some(9000));
    }

    #[test]
    fn issuer_returns_iss_claim() {
        let cs = make_claims(vec![(
            "iss",
            ClaimValue::String("https://example.com".into()),
        )]);
        assert_eq!(cs.issuer(), Some("https://example.com"));
    }
}
