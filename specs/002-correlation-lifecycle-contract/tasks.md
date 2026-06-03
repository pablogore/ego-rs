# Tasks: Correlation Lifecycle Contract

**Input**: [spec.md](spec.md), [plan.md](plan.md)

**Prerequisites**: `plan.md` (required), `spec.md` (required), `research.md` (completed), `quickstart.md` (completed)

**MVP Scope**: Update Persistence SPI contract documentation (spec 001) with explicit correlation lifecycle invariants — creation origin, propagation path, retry survival, no downstream regeneration. No code changes.

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

**Purpose**: Read existing spec 001 documentation to confirm pre-modification state.

- [x] T001 [P] Verify spec 001 spec.md identifies sections for lifecycle contract
      Action: Validate
      File: specs/001-persistence-spi/spec.md
      Section: Contract Invariants
      Outcome: Contract Invariants section located; location for "Correlation Lifecycle" subsection identified
      Validation: `grep -n "Contract Invariants" specs/001-persistence-spi/spec.md` returns a line

- [x] T002 [P] Verify spec 001 event-store contract exists
      Action: Validate
      File: specs/001-persistence-spi/contracts/event-store.md
      Section: Critical Invariants
      Outcome: EventStore contract file found; Critical Invariants section located
      Validation: `grep -n "Critical Invariants" specs/001-persistence-spi/contracts/event-store.md` returns a line

---

## Phase 2: Update Spec 001 Contract Documentation (US1, US2, US3)

**Purpose**: Add explicit lifecycle contract invariants for correlation_id to spec 001 documents. Satisfies FR-001 through FR-007.

**Validation**: All lifecycle rules appear in the updated documents.

- [x] T003 [US1] Add correlation lifecycle creation origin invariant to spec 001 spec.md
      Action: Modify
      File: specs/001-persistence-spi/spec.md
      Section: Contract Invariants — new "Correlation Lifecycle" subsection
      Outcome: New "Correlation Lifecycle" subsection with invariants: (1) correlation_id originates in CommandContext, (2) flows from CommandContext → Events → EventStore, (3) bound to command identity, (4) MUST survive retries, (5) MUST NOT be regenerated downstream, (6) `None` is valid and not auto-generated. Cross-references spec 004 and 005.
      Validation: `grep -c "Correlation Lifecycle" specs/001-persistence-spi/spec.md` returns 1

- [x] T004 [US2] Add retry survival invariant to lifecycle section
      Action: Modify
      File: specs/001-persistence-spi/spec.md
      Section: Contract Invariants — Correlation Lifecycle
      Outcome: Retry survival invariant states: correlation_id is bound to command identity, not execution attempt. All retries of the same logical command use identical correlation_id.
      Validation: `grep "survive\|retry\|attempt" specs/001-persistence-spi/spec.md | grep -i "correlation"` returns at least 1 line

- [x] T005 [US3] Add downstream propagation invariant to lifecycle section
      Action: Modify
      File: specs/001-persistence-spi/spec.md
      Section: Contract Invariants — Correlation Lifecycle
      Outcome: Downstream propagation invariant states: causally-related downstream commands carry the source event's correlation_id. No regeneration. Independent causality chains use `None`.
      Validation: `grep "downstream\|propagat\|regenerat" specs/001-persistence-spi/spec.md | grep -i "correlation"` returns at least 1 line

- [x] T006 [P] [US1] Update event-store contract with lifecycle propagation invariant
      Action: Modify
      File: specs/001-persistence-spi/contracts/event-store.md
      Section: Behavioral Contract — Critical Invariants
      Outcome: New invariant: "correlation_id flows from CommandContext through append to load without modification or regeneration." Lifecycle propagation path is documented.
      Validation: `grep "CommandContext\|lifecycle\|flows from" specs/001-persistence-spi/contracts/event-store.md | grep -i "correlation"` returns at least 1 line

---

## Phase 3: None Semantics (US4)

**Purpose**: Document that `correlation_id = None` is valid and the system never auto-generates. Satisfies FR-002 (Optionality) from spec 002.

- [x] T007 [US4] Add None semantics to lifecycle section
      Action: Modify
      File: specs/001-persistence-spi/spec.md
      Section: Contract Invariants — Correlation Lifecycle
      Outcome: Explicit invariant: "correlation_id = None means no traceability link. No layer SHALL auto-generate a correlation_id when None is provided."
      Validation: `grep "None\|auto-generat\|Optional" specs/001-persistence-spi/spec.md | grep -i "correlation"` returns at least 1 line

---

## Phase 4: Quickstart Validation

**Purpose**: Execute validation scenarios from quickstart.md to confirm all documentation updates are correct.

- [x] T008 Run quickstart Scenario 1 — verify lifecycle contract in spec 001 spec.md
      Action: Validate
      File: specs/001-persistence-spi/spec.md
      Section: Contract Invariants — Correlation Lifecycle
      Outcome: Four lifecycle rules present: creation origin, propagation path, retry survival, no downstream regeneration
      Validation: `grep -c "Correlation Lifecycle" specs/001-persistence-spi/spec.md` returns 1

- [x] T009 Run quickstart Scenario 2 — verify lifecycle propagation in event-store contract
      Action: Validate
      File: specs/001-persistence-spi/contracts/event-store.md
      Section: Behavioral Contract — Critical Invariants
      Outcome: Lifecycle propagation invariant documented in event-store contract
      Validation: `grep "CommandContext\|lifecycle" specs/001-persistence-spi/contracts/event-store.md | grep -i "correlation"` returns at least 1 line

- [x] T010 Run quickstart Scenario 3 — verify cross-references to specs 004 and 005
      Action: Validate
      File: specs/001-persistence-spi/spec.md
      Section: Correlation Lifecycle or Assumptions
      Outcome: Cross-references to Correlation Scope Boundary (004) and Correlation Semantic Boundary (005) present
      Validation: `grep "004\|005\|scope\|semantic" specs/001-persistence-spi/spec.md | grep -i "correlation"` returns at least 1 line

- [x] T011 Run quickstart Scenario 4 — no behavioral changes
      Action: Validate
      File: workspace root
      Section: test suite
      Outcome: All existing tests pass
      Validation: `cargo test` — exits 0

---

## Dependencies & Execution Order

```
Phase 1 (Foundational) → Phase 2 (Lifecycle Invariants) → Phase 3 (None Semantics) → Phase 4 (Validation)
```

### Parallel Opportunities

| Phase | [P] Tasks | Rationale |
|-------|-----------|-----------|
| Phase 1 | T001, T002 | Different files, independent inspection |
| Phase 2 | T006 | Independent of T003–T005 (different file) |

### Execution Strategy

1. **T001–T002** in parallel
2. **T003** (create lifecycle section), then **T004, T005** (add invariants to section)
3. **T006** in parallel with T003–T005 (different target file)
4. **T007** (None semantics — depends on T003 existing)
5. **T008–T011** in parallel (all independent validations)

---

## Definition of Done

- [x] `specs/001-persistence-spi/spec.md` has "Correlation Lifecycle" subsection under Contract Invariants
- [x] Four lifecycle rules documented: creation origin, propagation path, retry survival, no downstream regeneration
- [x] `correlation_id = None` semantics documented (valid, no auto-generation)
- [x] `specs/001-persistence-spi/contracts/event-store.md` has lifecycle propagation invariant
- [x] Cross-references to spec 004 and spec 005 present
- [x] No code changes — `cargo test` passes
- [x] quickstart.md all scenarios pass
