## ADDED Requirements

### Requirement: Runtime-mediated transport binding governance

Runtime-mediated transport exposure SHALL be governed by the Transport Binding Model (`specs/transport-binding-model/spec.md`).

Runtime Abstraction SHALL remain authoritative for:
- runtime capability semantics,
- runtime execution expectations,
- capability mediation.

Transport Binding Model SHALL remain authoritative for:
- transport exposure semantics,
- endpoint exposure binding,
- exposure descriptor binding,
- transport policy attachment.

Authority ownership MUST remain explicit and non-overlapping.

#### Scenario: Runtime-mediated transport exposure
- **WHEN** runtime capabilities expose a service interaction through a transport boundary
- **THEN** Runtime Abstraction SHALL govern runtime execution semantics and Transport Binding Model SHALL govern transport exposure semantics