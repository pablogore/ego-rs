# Research: Correlation Lifecycle Contract

**Phase**: 0 — Outline & Research | **Spec**: [spec.md](spec.md)

## Unknowns Assessment

No technical unknowns. The correlation lifecycle contract documents existing behavioral rules that the SPI implementation already follows.

## Design Decisions

### Decision 1: Lifecycle contract as Contract Invariants addition

- **Decision**: Add a new "Correlation Lifecycle" subsection to spec 001's Contract Invariants.
- **Rationale**: The lifecycle rules (creation in CommandContext, propagation to EventStore, retry survival, no downstream regeneration) are behavioral guarantees that all SPI implementations must satisfy.
- **Alternatives considered**: Separate lifecycle document. Rejected — fragmented correlation_id rules across files violates FR-005 (consolidated documentation).

### Decision 2: No code changes

- **Decision**: Documentation-only amendment.
- **Rationale**: The correlation_id behavior is already implemented and tested. The lifecycle contract makes the implicit rules explicit.
- **Alternatives considered**: Adding runtime assertions. Rejected per §A (Anti Over-Engineering) — no speculative enforcement.

## Recommendations

Proceed to Phase 1 (Design & Contracts).
