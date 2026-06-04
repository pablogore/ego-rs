use ego_domain::context::{
    AggregateId, CausationId, CorrelationId, EntityId, ExecutionContext, Metadata, RequestId,
    TenantId,
};

/// Runtime implementation of the domain [`ExecutionContext`] trait.
///
/// Carries identity, correlation, and metadata for the current execution.
/// Constructed by the runtime from the incoming message or envelope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandContext {
    aggregate_id: Option<AggregateId>,
    entity_id: Option<EntityId>,
    tenant_id: Option<TenantId>,
    correlation_id: Option<CorrelationId>,
    causation_id: Option<CausationId>,
    request_id: Option<RequestId>,
    metadata: Metadata,
}

impl CommandContext {
    pub fn new(
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

impl ExecutionContext for CommandContext {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn test_aggregate_id() -> AggregateId {
        AggregateId::new("agg-1").unwrap()
    }

    fn test_entity_id() -> EntityId {
        EntityId::new("ent-1").unwrap()
    }

    fn test_tenant_id() -> TenantId {
        TenantId::new("ten-1").unwrap()
    }

    fn test_correlation_id() -> CorrelationId {
        CorrelationId::new("corr-1").unwrap()
    }

    fn test_causation_id() -> CausationId {
        CausationId::new("caus-1").unwrap()
    }

    fn test_request_id() -> RequestId {
        RequestId::new("req-1").unwrap()
    }

    fn populated_metadata() -> Metadata {
        let mut m = Metadata::new();
        m.insert("key1".into(), "val1".into());
        m.insert("key2".into(), "val2".into());
        m
    }

    fn empty_context() -> CommandContext {
        CommandContext::new(None, None, None, None, None, None, Metadata::new())
    }

    fn full_context() -> CommandContext {
        CommandContext::new(
            Some(test_aggregate_id()),
            Some(test_entity_id()),
            Some(test_tenant_id()),
            Some(test_correlation_id()),
            Some(test_causation_id()),
            Some(test_request_id()),
            populated_metadata(),
        )
    }

    #[test]
    fn test_identity_fields_round_trip() {
        let ctx = full_context();
        assert_eq!(ctx.aggregate_id(), Some(&test_aggregate_id()));
        assert_eq!(ctx.entity_id(), Some(&test_entity_id()));
        assert_eq!(ctx.tenant_id(), Some(&test_tenant_id()));
    }

    #[test]
    fn test_identity_fields_none() {
        let ctx = empty_context();
        assert_eq!(ctx.aggregate_id(), None);
        assert_eq!(ctx.entity_id(), None);
        assert_eq!(ctx.tenant_id(), None);
    }

    #[test]
    fn test_correlation_fields_round_trip() {
        let ctx = full_context();
        assert_eq!(ctx.correlation_id(), Some(&test_correlation_id()));
        assert_eq!(ctx.causation_id(), Some(&test_causation_id()));
        assert_eq!(ctx.request_id(), Some(&test_request_id()));
    }

    #[test]
    fn test_correlation_fields_none() {
        let ctx = empty_context();
        assert_eq!(ctx.correlation_id(), None);
        assert_eq!(ctx.causation_id(), None);
        assert_eq!(ctx.request_id(), None);
    }

    #[test]
    fn test_metadata_populated() {
        let ctx = full_context();
        assert_eq!(ctx.metadata(), &populated_metadata());
    }

    #[test]
    fn test_metadata_empty() {
        let ctx = empty_context();
        assert!(ctx.metadata().is_empty());
    }

    #[test]
    fn test_all_fields_round_trip() {
        let ctx = full_context();
        assert_eq!(ctx.aggregate_id(), Some(&test_aggregate_id()));
        assert_eq!(ctx.entity_id(), Some(&test_entity_id()));
        assert_eq!(ctx.tenant_id(), Some(&test_tenant_id()));
        assert_eq!(ctx.correlation_id(), Some(&test_correlation_id()));
        assert_eq!(ctx.causation_id(), Some(&test_causation_id()));
        assert_eq!(ctx.request_id(), Some(&test_request_id()));
        assert_eq!(ctx.metadata(), &populated_metadata());
    }

    #[test]
    fn test_immutability() {
        let ctx = empty_context();
        let _ = ctx.aggregate_id();
        let _ = ctx.metadata();
    }

    #[test]
    fn test_clone_preserves_values() {
        let ctx = full_context();
        let cloned = ctx.clone();
        assert_eq!(ctx, cloned);
    }

    #[test]
    fn test_trait_impl() {
        let ctx = full_context();
        let trait_obj: &dyn ExecutionContext = &ctx;
        assert_eq!(trait_obj.aggregate_id(), Some(&test_aggregate_id()));
        assert!(trait_obj.metadata().contains_key("key1"));
    }
}
