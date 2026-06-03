# Correlation Semantic Boundary — Contract Boundary Requirements Quality

**Purpose**: Validate the quality of requirements defining correlation_id negative semantic boundaries
**Created**: 2026-06-03
**Feature**: [spec.md](../spec.md)

## Requirement Completeness

- [ ] CHK001 - Are all four negative semantic boundaries (security, correctness, ordering, deduplication) explicitly defined as functional requirements? [Completeness, Spec §FR-001–FR-004]
- [ ] CHK002 - Is the positive semantics of correlation_id (what it IS) also defined alongside the negative semantics as required by FR-005? [Gap, Spec §FR-005]
- [ ] CHK003 - Are requirements defined for what happens when the boundaries are violated (e.g., someone uses correlation_id in auth logic)? [Gap, Spec §Constraints]
- [ ] CHK004 - Is the relationship between each boundary and the existing SPI contracts (EventStore, Repository, Snapshot) explicitly mapped? [Completeness]
- [ ] CHK005 - Are the consequences of boundary violation documented for SPI implementors? [Gap]

## Requirement Clarity

- [ ] CHK006 - Is "NOT a security token" unambiguously scoped to exclude authentication, authorization, session management, and any cryptographic assumptions? [Clarity, Spec §FR-001]
- [ ] CHK007 - Is "NOT required for correctness" quantified by defining exactly which persistence operations succeed regardless of correlation_id presence? [Clarity, Spec §FR-002]
- [ ] CHK008 - Is "NOT used for ordering" precise about the sole ordering mechanism (stream version) and that correlation_id plays no role? [Clarity, Spec §FR-003]
- [ ] CHK009 - Is "NOT used for deduplication" precise about what separates deduplication from correlation_id (different key, different purpose)? [Clarity, Spec §FR-004]
- [ ] CHK010 - Is the term "consolidated section" in FR-005 defined with specific location criteria (which document, which section)? [Clarity, Spec §FR-005]
- [ ] CHK011 - Are the constraint statements in §Constraints clearly scoped to the SPI boundary versus application/infrastructure layers? [Clarity, Spec §Constraints]

## Requirement Consistency

- [ ] CHK012 - Do FR-001 through FR-004 align consistently with the Contract Invariants in spec 005? [Consistency, Spec §FR-001–FR-004 vs §Contract Invariants]
- [ ] CHK013 - Do the four negative boundaries conflict with any positive semantics defined in spec 001, 002, or 004? [Consistency, Cross-spec]
- [ ] CHK014 - Is the scope boundary consistent between spec 005 §Out of Scope and the Assumptions section? [Consistency, Spec §Out of Scope vs §Assumptions]
- [ ] CHK015 - Does "NOT required for correctness" (FR-002) align with the existing spec 001 assumption that correlation_id is optional? [Consistency, Spec §FR-002 vs Spec 001 §Assumptions]

## Acceptance Criteria Quality

- [ ] CHK016 - Can SC-001 ("identify four negative boundaries in under 2 minutes") be objectively measured? [Measurability, Spec §SC-001]
- [ ] CHK017 - Is SC-002 ("security audit finds zero cases") verifiable without implementation knowledge? [Measurability, Spec §SC-002]
- [ ] CHK018 - Can SC-003 ("events returned in append order") be tested independently of a specific backend? [Measurability, Spec §SC-003]
- [ ] CHK019 - Is SC-004 ("both events present on load") clearly distinct from deduplication logic? [Measurability, Spec §SC-004]

## Scenario Coverage

- [ ] CHK020 - Are requirements defined for the scenario where a downstream consumer intentionally uses correlation_id for deduplication (boundary violation)? [Coverage, Gap]
- [ ] CHK021 - Are requirements defined for the scenario where an SPI implementation stores correlation_id in a security-sensitive column? [Coverage, Gap]
- [ ] CHK022 - Are requirements defined for the interaction between the correlation_id boundary and the existing StoredEvent envelope? [Coverage, Spec §FR-018]
- [ ] CHK023 - Is the read-side consumer scenario (events loaded and interpreted by downstream handlers) addressed in the boundary definitions? [Coverage]

## Edge Case Coverage

- [ ] CHK024 - Are requirements defined for what happens when correlation_id is empty string (""), distinguishing it from None? [Edge Case, Spec §Edge Cases]
- [ ] CHK025 - Is the scenario where an external system sends a JWT-like string as correlation_id addressed? [Edge Case, Spec §Edge Cases]
- [ ] CHK026 - Are requirements defined for correlation_id collision (same value across different causation paths)? [Edge Case, Spec §Edge Cases]
- [ ] CHK027 - Is the scenario where a cache key uses correlation_id value addressed (acceptable for trace grouping)? [Edge Case, Spec §Edge Cases]

## Non-Functional Requirements

- [ ] CHK028 - Is the performance impact of correlation_id boundary enforcement (or lack thereof) specified? [Gap, NFR]
- [ ] CHK029 - Are security requirements clearly separated from correlation_id semantics (what handles auth if correlation_id doesn't)? [Completeness, Security]
- [ ] CHK030 - Are traceability assumptions about correlation_id documented for downstream consumers? [Assumption, Spec §Assumptions]

## Dependencies & Assumptions

- [ ] CHK031 - Is the dependency on spec 001 (StoredEvent envelope) explicitly documented as a prerequisite? [Dependency, Spec §Assumptions]
- [ ] CHK032 - Is the dependency on spec 002 (correlation lifecycle) and spec 004 (scope boundary) documented? [Dependency, Spec §Assumptions]
- [ ] CHK033 - Is the assumption that "SPI consumers are responsible for their own deduplication" validated or documented as a risk? [Assumption, Spec §Assumptions]
- [ ] CHK034 - Is the assumption that down-stream handlers will propagate correlation_id (not re-generate) consistent with the boundaries? [Assumption, Gap]

## Ambiguities & Conflicts

- [ ] CHK035 - Is there ambiguity about whether the boundaries apply to SPI implementations, SPI consumers, or both? [Ambiguity, Spec §Contract Invariants]
- [ ] CHK036 - Is there a conflict between FR-005 (consolidated section) and the existing spread of correlation_id mentions across spec 001? [Conflict, Spec §FR-005 vs Spec 001 structure]
- [ ] CHK037 - Is the boundary between "correlation_id is not a security token" and "the system must still track who issued a command" clearly delineated? [Ambiguity, Gap]
