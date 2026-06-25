use crate::context::{
    AggregateId, CausationId, CorrelationId, EntityId, Metadata, RequestId, TenantId,
};

/// Transport-neutral carrier for execution input.
///
/// Carries the payload (command, event, workflow message, etc.) alongside
/// identity, correlation, and metadata from the incoming message.
///
/// # Type parameters
///
/// - `P`: Payload type — determined by the execution model.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ExecutionEnvelope<P> {
    /// The input message payload.
    pub payload: P,
    /// Aggregate identity, if available.
    pub aggregate_id: Option<AggregateId>,
    /// Entity identity, if available.
    pub entity_id: Option<EntityId>,
    /// Tenant identity, if available.
    pub tenant_id: Option<TenantId>,
    /// Correlation identifier, if available.
    pub correlation_id: Option<CorrelationId>,
    /// Causation identifier, if available.
    pub causation_id: Option<CausationId>,
    /// Request identifier, if available.
    pub request_id: Option<RequestId>,
    /// Arbitrary key-value metadata.
    pub metadata: Metadata,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::{AggregateId, CorrelationId, EntityId, Metadata, TenantId};

    fn test_aggregate_id() -> AggregateId {
        AggregateId::new("agg-001").unwrap()
    }

    fn test_entity_id() -> EntityId {
        EntityId::new("ent-001").unwrap()
    }

    fn test_tenant_id() -> TenantId {
        TenantId::new("ten-001").unwrap()
    }

    fn test_correlation_id() -> CorrelationId {
        CorrelationId::new("corr-001").unwrap()
    }

    fn full_envelope() -> ExecutionEnvelope<String> {
        let mut meta = Metadata::new();
        meta.insert("key1".into(), "val1".into());
        ExecutionEnvelope {
            payload: "test-payload".into(),
            aggregate_id: Some(test_aggregate_id()),
            entity_id: Some(test_entity_id()),
            tenant_id: Some(test_tenant_id()),
            correlation_id: Some(test_correlation_id()),
            causation_id: Some(CausationId::new("caus-001").unwrap()),
            request_id: Some(RequestId::new("req-001").unwrap()),
            metadata: meta,
        }
    }

    #[test]
    fn envelope_construction_all_fields() {
        let envelope = full_envelope();
        assert_eq!(envelope.payload, "test-payload");
        assert_eq!(envelope.aggregate_id, Some(test_aggregate_id()));
        assert_eq!(envelope.entity_id, Some(test_entity_id()));
        assert_eq!(envelope.tenant_id, Some(test_tenant_id()));
        assert_eq!(envelope.correlation_id, Some(test_correlation_id()));
        assert_eq!(
            envelope.causation_id,
            Some(CausationId::new("caus-001").unwrap())
        );
        assert_eq!(
            envelope.request_id,
            Some(RequestId::new("req-001").unwrap())
        );
        assert_eq!(envelope.metadata.get("key1").unwrap(), "val1");
    }

    #[test]
    fn envelope_construction_identity_none() {
        let envelope = ExecutionEnvelope::<String> {
            payload: "test".into(),
            aggregate_id: None,
            entity_id: None,
            tenant_id: None,
            correlation_id: None,
            causation_id: None,
            request_id: None,
            metadata: Metadata::new(),
        };
        assert_eq!(envelope.aggregate_id, None);
        assert_eq!(envelope.entity_id, None);
        assert_eq!(envelope.tenant_id, None);
    }

    #[test]
    fn envelope_construction_correlation_none() {
        let envelope = ExecutionEnvelope::<String> {
            payload: "test".into(),
            aggregate_id: None,
            entity_id: None,
            tenant_id: None,
            correlation_id: None,
            causation_id: None,
            request_id: None,
            metadata: Metadata::new(),
        };
        assert_eq!(envelope.correlation_id, None);
        assert_eq!(envelope.causation_id, None);
        assert_eq!(envelope.request_id, None);
    }

    #[test]
    fn envelope_construction_metadata_empty() {
        let envelope = ExecutionEnvelope::<String> {
            payload: "test".into(),
            aggregate_id: None,
            entity_id: None,
            tenant_id: None,
            correlation_id: None,
            causation_id: None,
            request_id: None,
            metadata: Metadata::new(),
        };
        assert!(envelope.metadata.is_empty());
    }

    #[test]
    fn envelope_clone_preserves_values() {
        let envelope = full_envelope();
        let cloned = envelope.clone();
        assert_eq!(envelope, cloned);
    }

    #[test]
    fn envelope_debug_format() {
        let envelope = full_envelope();
        let debug = format!("{:?}", envelope);
        assert!(debug.contains("ExecutionEnvelope"));
        assert!(debug.contains("test-payload"));
    }

    #[test]
    fn envelope_equality() {
        let a = full_envelope();
        let b = full_envelope();
        assert_eq!(a, b);
    }

    #[test]
    fn envelope_equality_different_payload() {
        let a = full_envelope();
        let b = ExecutionEnvelope {
            payload: "different".into(),
            ..full_envelope()
        };
        assert_ne!(a, b);
    }

    #[test]
    fn envelope_serialization_round_trip() {
        let envelope = full_envelope();
        let json = serde_json::to_string(&envelope).unwrap();
        let deserialized: ExecutionEnvelope<String> = serde_json::from_str(&json).unwrap();
        assert_eq!(envelope, deserialized);
    }

    #[test]
    fn envelope_serialization_none_fields() {
        let envelope = ExecutionEnvelope::<String> {
            payload: "test".into(),
            aggregate_id: None,
            entity_id: None,
            tenant_id: None,
            correlation_id: None,
            causation_id: None,
            request_id: None,
            metadata: Metadata::new(),
        };
        let json = serde_json::to_string(&envelope).unwrap();
        let deserialized: ExecutionEnvelope<String> = serde_json::from_str(&json).unwrap();
        assert_eq!(envelope, deserialized);
        assert!(json.contains("null"));
    }

    #[test]
    fn envelope_works_with_different_payload_types() {
        let string_env = ExecutionEnvelope::<String> {
            payload: "hello".into(),
            aggregate_id: None,
            entity_id: None,
            tenant_id: None,
            correlation_id: None,
            causation_id: None,
            request_id: None,
            metadata: Metadata::new(),
        };
        assert_eq!(string_env.payload, "hello");

        #[derive(Debug, Clone, PartialEq, Eq)]
        struct MyPayload {
            value: u32,
        }

        let struct_env = ExecutionEnvelope::<MyPayload> {
            payload: MyPayload { value: 42 },
            aggregate_id: None,
            entity_id: None,
            tenant_id: None,
            correlation_id: None,
            causation_id: None,
            request_id: None,
            metadata: Metadata::new(),
        };
        assert_eq!(struct_env.payload.value, 42);
    }
}
