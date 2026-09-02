//! `id_type!` — the identity-type generator relocated from
//! `ego_domain::context` (CORE-PERSIST-A AD-3, `context.rs:7-54`).
//!
//! `#[macro_export]`ed so `ego-domain` can re-invoke it for its remaining
//! identity types (`AggregateId`, `EntityId`, `CorrelationId`,
//! `CausationId`, `RequestId`) instead of duplicating the definition — one
//! generator, not two. `TenantId`/`TenantIdError` are generated here;
//! `ego-domain` re-exports both at their original path
//! (`ego_domain::context::TenantId`, `ego_domain::TenantId`).

#[macro_export]
macro_rules! id_type {
    ($name:ident, $error:ident, $msg:expr) => {
        #[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
        #[serde(try_from = "String")]
        pub struct $name(String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Result<Self, $error> {
                let s = value.into();
                if s.trim().is_empty() {
                    Err($error)
                } else {
                    Ok(Self(s))
                }
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl TryFrom<String> for $name {
            type Error = $error;

            fn try_from(value: String) -> Result<Self, Self::Error> {
                Self::new(value)
            }
        }

        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub struct $error;

        impl std::fmt::Display for $error {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, $msg)
            }
        }

        impl std::error::Error for $error {}
    };
}
pub(crate) use id_type;

id_type!(TenantId, TenantIdError, "tenant_id must not be empty");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tenant_id_valid() {
        let id = TenantId::new("tenant-xyz").unwrap();
        assert_eq!(id.as_str(), "tenant-xyz");
    }

    #[test]
    fn test_tenant_id_empty_rejected() {
        let err = TenantId::new("").unwrap_err();
        assert_eq!(err, TenantIdError);
    }

    #[test]
    fn test_tenant_id_deserialize_valid() {
        let id: TenantId = serde_json::from_str("\"tenant-xyz\"").unwrap();
        assert_eq!(id.as_str(), "tenant-xyz");
    }

    #[test]
    fn test_tenant_id_deserialize_empty_rejected() {
        let result: Result<TenantId, _> = serde_json::from_str("\"\"");
        assert!(result.is_err());
    }

    #[test]
    fn test_tenant_id_deserialize_whitespace_rejected() {
        let result: Result<TenantId, _> = serde_json::from_str("\"   \"");
        assert!(result.is_err());
    }
}
