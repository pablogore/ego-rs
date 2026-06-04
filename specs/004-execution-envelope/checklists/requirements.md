# Specification Quality Checklist: Execution Envelope

**Feature**: [specs/004-execution-envelope/spec.md](spec.md)

## Checklist

- [ ] ExecutionEnvelope as generic struct (not trait)
- [ ] Payload type parameter P (execution-model agnostic)
- [ ] Identity fields: aggregate_id, entity_id, tenant_id (reusing 002 types)
- [ ] Correlation fields: correlation_id, causation_id, request_id (reusing 002 types)
- [ ] Metadata: HashMap<String, String> (reusing 002 types)
- [ ] All identity/correlation fields optional (Option<...>)
- [ ] No transport, actor, Tokio, or runtime types in envelope
- [ ] ExecutionContext constructable from envelope
- [ ] Runtime struct refactored to accept envelope
- [ ] TDD: test-first for all phases
