# Research: Correlation Semantic Boundary

**Phase**: 0 — Outline & Research | **Spec**: [spec.md](spec.md)

## Unknowns Assessment

No technical unknowns exist for this feature. The spec defines four explicit negative semantic boundaries for correlation_id (security, correctness, ordering, deduplication) that are clear and unambiguous.

## Design Decisions

### Decision 1: Documentation-only amendment

- **Decision**: No code changes. All requirements are satisfied by updating existing contract documentation.
- **Rationale**: The existing `StoredEvent<E>` data structure (`correlation_id: Option<String>`) already behaves correctly — it is an opaque optional string that does not influence security, correctness, ordering, or deduplication. The gap was in explicit documentation, not implementation.
- **Alternatives considered**: Adding runtime assertions or type-level constraints (e.g., newtype wrapper). Rejected because the spec requires explicit documentation (FR-005), not behavioral changes. Adding runtime enforcement would violate Anti Over-Engineering (§A).

### Decision 2: In-place modification of existing spec 001 documents

- **Decision**: Modify `specs/001-persistence-spi/spec.md`, `contracts/event-store.md`, and `data-model.md` directly.
- **Rationale**: The correlation_id contract is already defined in these documents. Adding negative semantics alongside existing positive semantics keeps the contract cohesive per FR-005 ("consolidated section").
- **Alternatives considered**: Creating a separate "correlation_id boundaries" document. Rejected because it fragments the contract and violates §H (Modify Before Duplicate).

## Recommendations

Proceed directly to Phase 1 (Design & Contracts). No further research is required.
