# Tasks: Correlation Semantic Boundary

**Input**: [spec.md](spec.md), [plan.md](plan.md)

**Prerequisites**: `plan.md` (required), `spec.md` (required), `research.md` (completed), `quickstart.md` (completed)

**MVP Scope**: Update Persistence SPI contract documentation (spec 001) with explicit "what correlation_id is NOT" negative semantic boundaries. No code changes.

**Deferred**: N/A — documentation-only amendment.

**Design**: [plan.md](plan.md)

**Constitution**: `.speckit/constitution.md` v2.0.0

---

## Task Format

```
- [ ] TXXX [P] [USN] Short description
      Action: Create | Modify | Refactor | Delete
      File: path/to/file.md
      Section: section name
      Outcome: what exists after completion
      Validation: command that proves completion
```

---

## Phase 1: Foundational — Load Spec 001 Documents

**Purpose**: Read existing spec 001 documentation to confirm pre-modification state. All tasks are verification-only.

**Validation**: Manual inspection — confirm each file exists and has not been modified yet.

- [ ] T001 [P] Verify spec 001 spec.md exists and identify sections to update
      Action: Validate
      File: specs/001-persistence-spi/spec.md
      Section: FR-018, Contract Invariants
      Outcome: FR-018 (Event Envelope) located; Contract Invariants section identified; assumption entries for correlation_id located
      Validation: `grep -n "FR-018" specs/001-persistence-spi/spec.md` returns a line

- [ ] T002 [P] Verify spec 001 event-store contract exists
      Action: Validate
      File: specs/001-persistence-spi/contracts/event-store.md
      Section: Critical Invariants
      Outcome: correlation_id entry found under Critical Invariants; section for semantic boundaries identified
      Validation: `grep -n "correlation_id" specs/001-persistence-spi/contracts/event-store.md` returns lines

- [ ] T003 [P] Verify spec 001 data-model.md exists
      Action: Validate
      File: specs/001-persistence-spi/data-model.md
      Section: StoredEvent
      Outcome: StoredEvent entity found with correlation_id field documentation
      Validation: `grep -n "StoredEvent" specs/001-persistence-spi/data-model.md` returns a line

---

## Phase 2: Update Spec 001 Contract Documentation (US1)

**Purpose**: Add explicit negative semantic boundaries for correlation_id to all three contract documents. Satisfies FR-001, FR-002, FR-003, FR-004, FR-005.

**Validation**: All four "NOT" boundaries appear in each document.

- [ ] T004 [US1] Add Correlation ID Semantic Boundaries section to spec 001 spec.md
      Action: Modify
      File: specs/001-persistence-spi/spec.md
      Section: Contract Invariants — after Tenant Isolation
      Outcome: New "Correlation ID Semantic Boundaries" subsection with four sub-sections (Security, Correctness, Ordering, Deduplication). Each sub-section contains the relevant "MUST NOT" / "NOT" rules from spec 005 FR-001 through FR-004. FR-018 updated to reference semantic boundaries.
      Validation: `grep -c "Correlation ID Semantic Boundaries" specs/001-persistence-spi/spec.md` returns 1

- [ ] T005 [US1] Add Correlation ID Semantic Boundaries section to event-store contract
      Action: Modify
      File: specs/001-persistence-spi/contracts/event-store.md
      Section: Behavioral Contract — after Critical Invariants
      Outcome: New "Correlation ID Semantic Boundaries" section under Behavioral Contract with four "NOT" statements: NOT a security token, NOT required for correctness, NOT used for ordering, NOT used for deduplication.
      Validation: `grep -c "Semantic Boundaries" specs/001-persistence-spi/contracts/event-store.md` returns 1

- [ ] T006 [US1] Add semantic boundaries note to data-model StoredEvent
      Action: Modify
      File: specs/001-persistence-spi/data-model.md
      Section: StoredEvent
      Outcome: StoredEvent entity includes "Semantic boundaries" entry listing the four "NOT" statements.
      Validation: `grep -c "Semantic boundaries" specs/001-persistence-spi/data-model.md` returns 1

---

## Phase 3: Security Validation (US2)

**Purpose**: Verify no existing code or documentation treats correlation_id as a security token. Satisfies FR-001.

**Validation**: `grep` searches confirm no security misuse.

- [ ] T007 [US2] Audit codebase for correlation_id security misuse
      Action: Validate
      File: workspace root (all crates)
      Section: N/A — security audit
      Outcome: Zero cases found where correlation_id is used for authentication, authorization, session management, or access control decisions
      Validation: `grep -rn "correlation_id" crates/ --include="*.rs" | grep -iv "test\|mock\|stored_event\|Option"` — no security-related hits

- [ ] T008 [US2] Verify spec 001 contract tests do not use correlation_id for security
      Action: Validate
      File: crates/infrastructure/tests/common/mod.rs (if exists)
      Section: event_store_contract_tests
      Outcome: Correlation_id test scenarios verify preservation and optionality only — no security semantics
      Validation: `grep -A5 "correlation_id" crates/infrastructure/tests/common/mod.rs` — test scenarios are about preservation, not security

---

## Phase 4: Ordering Documentation (US3)

**Purpose**: Confirm event ordering invariants explicitly state correlation_id is not an ordering mechanism. Satisfies FR-003.

- [ ] T009 [US3] Verify ordering invariants exclude correlation_id
      Action: Validate
      File: specs/001-persistence-spi/spec.md
      Section: Contract Invariants — Ordering
      Outcome: Ordering subsection includes explicit statement that correlation_id does not influence event order
      Validation: `grep -A10 "### Ordering" specs/001-persistence-spi/spec.md | grep -c "correlation"` returns at least 1

- [ ] T010 [US3] Verify event-store contract ordering invariant
      Action: Validate
      File: specs/001-persistence-spi/contracts/event-store.md
      Section: Behavioral Contract — Semantic Boundaries
      Outcome: "NOT used for ordering" boundary statement is present
      Validation: `grep "NOT used for ordering" specs/001-persistence-spi/contracts/event-store.md` returns a line

---

## Phase 5: Deduplication Documentation (US4)

**Purpose**: Confirm deduplication invariants explicitly state correlation_id is not a deduplication key. Satisfies FR-004.

- [ ] T011 [US4] Verify deduplication invariants exclude correlation_id
      Action: Validate
      File: specs/001-persistence-spi/spec.md
      Section: Contract Invariants — Deduplication
      Outcome: Deduplication subsection includes explicit statement that correlation_id is not an idempotency key and multiple events MAY share the same correlation_id
      Validation: `grep -A10 "### Deduplication" specs/001-persistence-spi/spec.md | grep -c "correlation"` returns at least 1

- [ ] T012 [US4] Verify event-store contract deduplication boundary
      Action: Validate
      File: specs/001-persistence-spi/contracts/event-store.md
      Section: Behavioral Contract — Semantic Boundaries
      Outcome: "NOT used for deduplication" boundary statement is present
      Validation: `grep "NOT used for deduplication" specs/001-persistence-spi/contracts/event-store.md` returns a line

---

## Phase 6: Consolidated Documentation (FR-005)

**Purpose**: Ensure the correlation_id contract documents both positive and negative semantics in a consolidated section. Satisfies FR-005.

- [ ] T013 [FR-005] Verify consolidated correlation_id contract documentation
      Action: Validate
      File: specs/001-persistence-spi/spec.md
      Section: Contract Invariants — Correlation ID Semantic Boundaries
      Outcome: A single section under Contract Invariants contains both the positive semantics (from existing FR-018 + invariants) and the negative semantics (four boundaries). The section is cohesive without fragmentation across multiple locations.
      Validation: Read `specs/001-persistence-spi/spec.md` — "Correlation ID Semantic Boundaries" section has the four negative boundaries, and FR-018 references the boundaries

---

## Phase 7: Quickstart Validation

**Purpose**: Execute the validation scenarios from quickstart.md to confirm all documentation updates are correct and consistent.

- [ ] T014 Run quickstart.md Scenario 1 — verify negative semantics in spec 001 spec.md
      Action: Validate
      File: specs/001-persistence-spi/spec.md
      Section: Correlation ID Semantic Boundaries
      Outcome: All four boundaries present (Security, Correctness, Ordering, Deduplication) with explicit "MUST NOT" / "NOT" language
      Validation: `grep "NOT" specs/001-persistence-spi/spec.md | grep -i "correlation" | wc -l` returns >= 4

- [ ] T015 Run quickstart.md Scenario 2 — verify boundaries in event-store contract
      Action: Validate
      File: specs/001-persistence-spi/contracts/event-store.md
      Section: Correlation ID Semantic Boundaries
      Outcome: Four "NOT" boundary statements present
      Validation: `grep "NOT" specs/001-persistence-spi/contracts/event-store.md | grep -i correlation | wc -l` returns >= 4

- [ ] T016 Run quickstart.md Scenario 3 — verify boundaries in data-model
      Action: Validate
      File: specs/001-persistence-spi/data-model.md
      Section: StoredEvent
      Outcome: StoredEvent includes semantic boundaries note with four "NOT" statements
      Validation: `grep "NOT" specs/001-persistence-spi/data-model.md | grep -i correlation | wc -l` returns >= 4

- [ ] T017 Run quickstart.md Scenario 4 — verify no behavioral changes
      Action: Validate
      File: workspace root
      Section: test suite
      Outcome: All existing tests pass — no behavioral changes introduced
      Validation: `cargo test` — exits 0

---

## Dependencies & Execution Order

```
Phase 1 (Foundational) → Phase 2 (US1 Documentation) → Phase 3 (US2 Security) → Phase 4 (US3 Ordering) → Phase 5 (US4 Deduplication) → Phase 6 (FR-005 Consolidation) → Phase 7 (Validation)
```

### Parallel Opportunities

| Phase | [P] Tasks | Rationale |
|-------|-----------|-----------|
| Phase 1 | T001, T002, T003 | Different files, independent inspection |
| Phase 2 | T004, T005, T006 | Different spec 001 files, independent edits |
| Phase 3 | T007, T008 | Independent searches |
| Phase 4 | T009, T010 | Independent grep checks |
| Phase 5 | T011, T012 | Independent grep checks |
| Phase 7 | T014, T015, T016, T017 | Independent validations |

### Execution Strategy

1. **T001–T003** in parallel (verify starting state)
2. **T004, T005, T006** in parallel (update all three spec 001 documents)
3. **T007, T008** in parallel (security audit — independent of doc changes)
4. **T009, T010** in parallel (ordering verification)
5. **T011, T012** in parallel (deduplication verification)
6. **T013** (consolidation check — depends on all doc updates)
7. **T014, T015, T016, T017** in parallel (final validation)
8. **All phases can be re-executed in any order after Phase 2** — documentation edits are independent

---

## Definition of Done

- [ ] `specs/001-persistence-spi/spec.md` has "Correlation ID Semantic Boundaries" section with four sub-sections
- [ ] `specs/001-persistence-spi/contracts/event-store.md` has "Correlation ID Semantic Boundaries" section
- [ ] `specs/001-persistence-spi/data-model.md` has semantic boundaries note on StoredEvent
- [ ] FR-018 in spec 001 references correlation_id semantic boundaries
- [ ] Codebase audit confirms correlation_id is never used for security decisions
- [ ] Ordering and deduplication invariants explicitly exclude correlation_id
- [ ] Correlation_id contract has both positive and negative semantics in a consolidated section
- [ ] No code changes — `cargo test` passes with no changes to test suite
- [ ] quickstart.md all scenarios pass

---

## Notes

- Per `.speckit/constitution.md` §F: every task includes Action, File, Section, Outcome, Validation
- Per `.speckit/constitution.md` §A: documentation changes are the minimal necessary — no speculative abstractions
- Per `.speckit/constitution.md` §H: existing spec 001 documents are modified (not duplicated)
- No code changes, no new traits, no new types — this is a documentation-only amendment
- All four "NOT" boundaries MUST appear in all three spec 001 documents for consistency
- The spec 001 Contract Invariants "Correlation ID Semantic Boundaries" section is the single source of truth (FR-005)
