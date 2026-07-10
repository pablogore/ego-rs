use std::collections::HashMap;

// ---------------------------------------------------------------------------
// Identity types
// ---------------------------------------------------------------------------

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

id_type!(
    AggregateId,
    AggregateIdError,
    "aggregate_id must not be empty"
);
id_type!(EntityId, EntityIdError, "entity_id must not be empty");
id_type!(TenantId, TenantIdError, "tenant_id must not be empty");
id_type!(
    CorrelationId,
    CorrelationIdError,
    "correlation_id must not be empty"
);
id_type!(
    CausationId,
    CausationIdError,
    "causation_id must not be empty"
);
id_type!(RequestId, RequestIdError, "request_id must not be empty");

/// Arbitrary key-value metadata attached to an execution context.
pub type Metadata = HashMap<String, String>;

#[cfg(test)]
mod tests {
    use super::*;

    // -----------------------------------------------------------------------
    // Identity type tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_aggregate_id_valid() {
        let id = AggregateId::new("agg-123").unwrap();
        assert_eq!(id.as_str(), "agg-123");
    }

    #[test]
    fn test_aggregate_id_empty_rejected() {
        let err = AggregateId::new("").unwrap_err();
        assert_eq!(err, AggregateIdError);
    }

    #[test]
    fn test_aggregate_id_whitespace_rejected() {
        let err = AggregateId::new("   ").unwrap_err();
        assert_eq!(err, AggregateIdError);
    }

    #[test]
    fn test_aggregate_id_deserialize_validates() {
        let id: AggregateId = serde_json::from_str("\"agg-123\"").unwrap();
        assert_eq!(id.as_str(), "agg-123");
        assert!(serde_json::from_str::<AggregateId>("\"\"").is_err());
        assert!(serde_json::from_str::<AggregateId>("\"   \"").is_err());
    }

    #[test]
    fn test_entity_id_valid() {
        let id = EntityId::new("ent-456").unwrap();
        assert_eq!(id.as_str(), "ent-456");
    }

    #[test]
    fn test_entity_id_empty_rejected() {
        let err = EntityId::new("").unwrap_err();
        assert_eq!(err, EntityIdError);
    }

    #[test]
    fn test_entity_id_deserialize_validates() {
        let id: EntityId = serde_json::from_str("\"ent-456\"").unwrap();
        assert_eq!(id.as_str(), "ent-456");
        assert!(serde_json::from_str::<EntityId>("\"\"").is_err());
        assert!(serde_json::from_str::<EntityId>("\"   \"").is_err());
    }

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

    #[test]
    fn test_correlation_id_valid() {
        let id = CorrelationId::new("corr-789").unwrap();
        assert_eq!(id.as_str(), "corr-789");
    }

    #[test]
    fn test_correlation_id_empty_rejected() {
        let err = CorrelationId::new("").unwrap_err();
        assert_eq!(err, CorrelationIdError);
    }

    #[test]
    fn test_correlation_id_deserialize_validates() {
        let id: CorrelationId = serde_json::from_str("\"corr-789\"").unwrap();
        assert_eq!(id.as_str(), "corr-789");
        assert!(serde_json::from_str::<CorrelationId>("\"\"").is_err());
        assert!(serde_json::from_str::<CorrelationId>("\"   \"").is_err());
    }

    #[test]
    fn test_causation_id_valid() {
        let id = CausationId::new("cause-001").unwrap();
        assert_eq!(id.as_str(), "cause-001");
    }

    #[test]
    fn test_causation_id_empty_rejected() {
        let err = CausationId::new("").unwrap_err();
        assert_eq!(err, CausationIdError);
    }

    #[test]
    fn test_causation_id_deserialize_validates() {
        let id: CausationId = serde_json::from_str("\"cause-001\"").unwrap();
        assert_eq!(id.as_str(), "cause-001");
        assert!(serde_json::from_str::<CausationId>("\"\"").is_err());
        assert!(serde_json::from_str::<CausationId>("\"   \"").is_err());
    }

    #[test]
    fn test_request_id_valid() {
        let id = RequestId::new("req-999").unwrap();
        assert_eq!(id.as_str(), "req-999");
    }

    #[test]
    fn test_request_id_empty_rejected() {
        let err = RequestId::new("").unwrap_err();
        assert_eq!(err, RequestIdError);
    }

    #[test]
    fn test_request_id_deserialize_validates() {
        let id: RequestId = serde_json::from_str("\"req-999\"").unwrap();
        assert_eq!(id.as_str(), "req-999");
        assert!(serde_json::from_str::<RequestId>("\"\"").is_err());
        assert!(serde_json::from_str::<RequestId>("\"   \"").is_err());
    }

    #[test]
    fn test_id_equality() {
        let a = AggregateId::new("foo").unwrap();
        let b = AggregateId::new("foo").unwrap();
        let c = AggregateId::new("bar").unwrap();
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn test_id_clone() {
        let a = AggregateId::new("clone-me").unwrap();
        let b = a.clone();
        assert_eq!(a, b);
    }

    #[test]
    fn test_id_debug() {
        let id = AggregateId::new("debug-test").unwrap();
        let debug = format!("{:?}", id);
        assert!(debug.contains("AggregateId"));
        assert!(debug.contains("debug-test"));
    }

    #[test]
    fn test_id_hash() {
        use std::collections::HashSet;
        let mut set = HashSet::new();
        set.insert(AggregateId::new("hash-test").unwrap());
        set.insert(AggregateId::new("hash-test").unwrap());
        assert_eq!(set.len(), 1);
    }
}
