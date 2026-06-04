use std::collections::HashMap;

// ---------------------------------------------------------------------------
// Identity types
// ---------------------------------------------------------------------------

macro_rules! id_type {
    ($name:ident, $error:ident, $msg:expr) => {
        #[derive(Debug, Clone, PartialEq, Eq, Hash)]
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

id_type!(AggregateId, AggregateIdError, "aggregate_id must not be empty");
id_type!(EntityId, EntityIdError, "entity_id must not be empty");
id_type!(TenantId, TenantIdError, "tenant_id must not be empty");
id_type!(CorrelationId, CorrelationIdError, "correlation_id must not be empty");
id_type!(CausationId, CausationIdError, "causation_id must not be empty");
id_type!(RequestId, RequestIdError, "request_id must not be empty");

/// Arbitrary key-value metadata attached to an execution context.
pub type Metadata = HashMap<String, String>;

// ---------------------------------------------------------------------------
// ExecutionContext trait
// ---------------------------------------------------------------------------

/// Read-only execution context for all execution models.
///
/// Carries identity, correlation, and metadata from the incoming message.
/// Implementations are provided by runtime crates. The trait is domain-owned
/// and runtime-neutral — no Tokio, async, or runtime-specific types.
pub trait ExecutionContext {
    fn aggregate_id(&self) -> Option<&AggregateId>;
    fn entity_id(&self) -> Option<&EntityId>;
    fn tenant_id(&self) -> Option<&TenantId>;
    fn correlation_id(&self) -> Option<&CorrelationId>;
    fn causation_id(&self) -> Option<&CausationId>;
    fn request_id(&self) -> Option<&RequestId>;
    fn metadata(&self) -> &Metadata;
}

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

    // -----------------------------------------------------------------------
    // ExecutionContext trait — concrete test implementation
    // -----------------------------------------------------------------------

    struct TestContext {
        aggregate_id: Option<AggregateId>,
        entity_id: Option<EntityId>,
        tenant_id: Option<TenantId>,
        correlation_id: Option<CorrelationId>,
        causation_id: Option<CausationId>,
        request_id: Option<RequestId>,
        metadata: Metadata,
    }

    impl TestContext {
        fn new(
            aggregate_id: Option<AggregateId>,
            entity_id: Option<EntityId>,
            tenant_id: Option<TenantId>,
            correlation_id: Option<CorrelationId>,
            causation_id: Option<CausationId>,
            request_id: Option<RequestId>,
            metadata: Metadata,
        ) -> Self {
            Self {
                aggregate_id,
                entity_id,
                tenant_id,
                correlation_id,
                causation_id,
                request_id,
                metadata,
            }
        }
    }

    impl ExecutionContext for TestContext {
        fn aggregate_id(&self) -> Option<&AggregateId> {
            self.aggregate_id.as_ref()
        }
        fn entity_id(&self) -> Option<&EntityId> {
            self.entity_id.as_ref()
        }
        fn tenant_id(&self) -> Option<&TenantId> {
            self.tenant_id.as_ref()
        }
        fn correlation_id(&self) -> Option<&CorrelationId> {
            self.correlation_id.as_ref()
        }
        fn causation_id(&self) -> Option<&CausationId> {
            self.causation_id.as_ref()
        }
        fn request_id(&self) -> Option<&RequestId> {
            self.request_id.as_ref()
        }
        fn metadata(&self) -> &Metadata {
            &self.metadata
        }
    }

    #[test]
    fn test_context_identity_round_trip() {
        let agg = AggregateId::new("agg-1").unwrap();
        let ent = EntityId::new("ent-1").unwrap();
        let ten = TenantId::new("ten-1").unwrap();
        let ctx = TestContext::new(Some(agg.clone()), Some(ent.clone()), Some(ten.clone()), None, None, None, Metadata::new());
        assert_eq!(ctx.aggregate_id(), Some(&agg));
        assert_eq!(ctx.entity_id(), Some(&ent));
        assert_eq!(ctx.tenant_id(), Some(&ten));
    }

    #[test]
    fn test_context_identity_none() {
        let ctx = TestContext::new(None, None, None, None, None, None, Metadata::new());
        assert_eq!(ctx.aggregate_id(), None);
        assert_eq!(ctx.entity_id(), None);
        assert_eq!(ctx.tenant_id(), None);
    }

    #[test]
    fn test_context_correlation_round_trip() {
        let corr = CorrelationId::new("corr-1").unwrap();
        let caus = CausationId::new("caus-1").unwrap();
        let req = RequestId::new("req-1").unwrap();
        let ctx = TestContext::new(None, None, None, Some(corr.clone()), Some(caus.clone()), Some(req.clone()), Metadata::new());
        assert_eq!(ctx.correlation_id(), Some(&corr));
        assert_eq!(ctx.causation_id(), Some(&caus));
        assert_eq!(ctx.request_id(), Some(&req));
    }

    #[test]
    fn test_context_correlation_none() {
        let ctx = TestContext::new(None, None, None, None, None, None, Metadata::new());
        assert_eq!(ctx.correlation_id(), None);
        assert_eq!(ctx.causation_id(), None);
        assert_eq!(ctx.request_id(), None);
    }

    #[test]
    fn test_context_metadata_populated() {
        let mut meta = Metadata::new();
        meta.insert("key1".into(), "val1".into());
        meta.insert("key2".into(), "val2".into());
        let ctx = TestContext::new(None, None, None, None, None, None, meta.clone());
        assert_eq!(ctx.metadata(), &meta);
    }

    #[test]
    fn test_context_metadata_empty() {
        let ctx = TestContext::new(None, None, None, None, None, None, Metadata::new());
        assert!(ctx.metadata().is_empty());
    }

    #[test]
    fn test_context_all_fields() {
        let agg = AggregateId::new("agg").unwrap();
        let ent = EntityId::new("ent").unwrap();
        let ten = TenantId::new("ten").unwrap();
        let corr = CorrelationId::new("corr").unwrap();
        let caus = CausationId::new("caus").unwrap();
        let req = RequestId::new("req").unwrap();
        let mut meta = Metadata::new();
        meta.insert("k".into(), "v".into());

        let ctx = TestContext::new(
            Some(agg.clone()), Some(ent.clone()), Some(ten.clone()),
            Some(corr.clone()), Some(caus.clone()), Some(req.clone()),
            meta.clone(),
        );

        assert_eq!(ctx.aggregate_id(), Some(&agg));
        assert_eq!(ctx.entity_id(), Some(&ent));
        assert_eq!(ctx.tenant_id(), Some(&ten));
        assert_eq!(ctx.correlation_id(), Some(&corr));
        assert_eq!(ctx.causation_id(), Some(&caus));
        assert_eq!(ctx.request_id(), Some(&req));
        assert_eq!(ctx.metadata(), &meta);
    }

    #[test]
    fn test_context_is_read_only() {
        let ctx = TestContext::new(None, None, None, None, None, None, Metadata::new());
        let _ = ctx.aggregate_id();
        let _ = ctx.metadata();
    }

    #[test]
    fn test_trait_object_compatible() {
        let ctx = TestContext::new(None, None, None, None, None, None, Metadata::new());
        let trait_obj: &dyn ExecutionContext = &ctx;
        assert!(trait_obj.metadata().is_empty());
    }
}
