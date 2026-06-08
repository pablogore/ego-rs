# Specification Quality Checklist: Execution Envelope

**Feature**: [specs/004-execution-envelope/spec.md](spec.md)

## Checklist

- [x] ExecutionEnvelope as generic struct (not trait)
- [x] Payload type parameter P (execution-model agnostic)
- [x] Identity fields: aggregate_id, entity_id, tenant_id (reusing 002 types)
- [x] Correlation fields: correlation_id, causation_id, request_id (reusing 002 types)
- [x] Metadata: HashMap<String, String> (reusing 002 types)
- [x] All identity/correlation fields optional (Option<...>)
- [x] No transport, actor, Tokio, or runtime types in envelope
- [x] ExecutionContext constructable from envelope
- [x] Runtime struct refactored to accept envelope
- [ ] TDD: test-first for all phases
