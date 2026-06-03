# Research: Correlation Scope Boundary

**Phase**: 0 — Outline & Research | **Spec**: [spec.md](spec.md)

## Unknowns Assessment

No technical unknowns. The scope boundary simply documents the existing architecture: Repository and Snapshot contracts already have no correlation_id in their trait signatures. This spec makes the implicit boundary explicit.

## Design Decisions

### Decision 1: Explicit "not a concern" statements per contract

- **Decision**: Each contract (Repository, Snapshot) gets an explicit statement that correlation_id is out of scope.
- **Rationale**: Prevents developers from wondering whether correlation_id should be added to these contracts. The "why" is documented: correlation_id is exclusively an event stream concern.
- **Alternatives considered**: Single centralized scope document. Rejected — per §H (Modify Before Duplicate), editing existing contracts keeps each contract's scope self-contained.

### Decision 2: No cross-contract test changes

- **Decision**: Existing contract tests are sufficient.
- **Rationale**: The tests already verify Repository and Snapshot without correlation_id. Adding tests that assert "no correlation_id" would test a negative, which is not actionable.
- **Alternatives considered**: Adding assertions that Repository/Snapshot tests do not reference correlation_id. Rejected — ceremonial, no behavioral value.

## Recommendations

Proceed to Phase 1 (Design & Contracts).
