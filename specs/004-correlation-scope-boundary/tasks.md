# Tasks: Correlation Scope Boundary

**Input**: [spec.md](spec.md), [plan.md](plan.md)

**Prerequisites**: `plan.md` (required), `spec.md` (required), `research.md` (completed), `quickstart.md` (completed)

**MVP Scope**: Update Persistence SPI contract documentation (spec 001) with explicit correlation_id ownership boundaries — EventStore owns it, Repository and Snapshot explicitly exclude it. No code changes.

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

- [x] T001 [P] Verify spec 001 spec.md and identify sections for scope boundary
      Action: Validate
      File: specs/001-persistence-spi/spec.md
      Section: Contract Invariants, Key Entities
      Outcome: EventStore, Repository, Snapshot contract sections located; correlation_id mentions identified
      Validation: `grep -n "EventStore\|Repository\|Snapshot\|correlation_id" specs/001-persistence-spi/spec.md | head -10` returns lines

- [x] T002 [P] Verify spec 001 repository contract exists
      Action: Validate
      File: specs/001-persistence-spi/contracts/repository.md
      Section: Trait Contract, Behavioral Contract
      Outcome: Repository contract file found; trait signature confirmed without correlation_id
      Validation: `grep "correlation_id" specs/001-persistence-spi/contracts/repository.md` exits 1 (no matches)

- [x] T003 [P] Verify spec 001 snapshot contract exists
      Action: Validate
      File: specs/001-persistence-spi/contracts/snapshot.md
      Section: Trait Contract, Behavioral Contract
      Outcome: Snapshot contract file found; trait signature confirmed without correlation_id
      Validation: `grep "correlation_id" specs/001-persistence-spi/contracts/snapshot.md` exits 1 (no matches)

---

## Phase 2: Scope Boundary Documentation (US1, US2, US3)

**Purpose**: Add explicit ownership boundaries for correlation_id across all three contracts. Satisfies FR-001 through FR-005.

**Validation**: Each contract explicitly states its relationship to correlation_id.

- [x] T004 [US1] Add scope boundary section to spec 001 spec.md
      Action: Modify
      File: specs/001-persistence-spi/spec.md
      Section: Contract Invariants — new "Correlation Scope Boundary" subsection
      Outcome: New "Correlation Scope Boundary" subsection stating: (1) EventStore owns correlation_id, (2) Repository is correlation_id-agnostic, (3) Snapshot is correlation_id-agnostic, (4) No operation outside EventStore SHALL accept or return correlation_id.
      Validation: `grep -c "Scope Boundary\|scope boundary" specs/001-persistence-spi/spec.md` returns at least 1

- [x] T005 [P] [US2] Add "not a concern" statement to repository contract
      Action: Modify
      File: specs/001-persistence-spi/contracts/repository.md
      Section: Behavioral Contract — Critical Invariants
      Outcome: New invariant: "correlation_id is NOT a Repository concern. Repository operations are correlation_id-agnostic. Correlation_id is exclusively owned by the EventStore."
      Validation: `grep -c "correlation_id\|not a Repository concern\|correlation_id-agnostic" specs/001-persistence-spi/contracts/repository.md` returns at least 1

- [x] T006 [P] [US3] Add "not a concern" statement to snapshot contract
      Action: Modify
      File: specs/001-persistence-spi/contracts/snapshot.md
      Section: Behavioral Contract — Critical Invariants
      Outcome: New invariant: "correlation_id is NOT a Snapshot concern. Snapshot operations are correlation_id-agnostic. Correlation_id is exclusively owned by the EventStore."
      Validation: `grep -c "correlation_id\|not a Snapshot concern\|correlation_id-agnostic" specs/001-persistence-spi/contracts/snapshot.md` returns at least 1

- [x] T007 [US1] Add ownership statement to event-store contract
      Action: Modify
      File: specs/001-persistence-spi/contracts/event-store.md
      Section: Behavioral Contract — Critical Invariants
      Outcome: Updated invariant: "correlation_id is exclusively owned by the EventStore. Repository and Snapshot contracts do not participate in the correlation lifecycle."
      Validation: `grep "exclusively owned\|Repository\|Snapshot" specs/001-persistence-spi/contracts/event-store.md | grep -i "correlation"` returns at least 1 line

---

## Phase 3: Quickstart Validation

**Purpose**: Execute validation scenarios from quickstart.md.

- [x] T008 Run quickstart Scenario 1 — Repository states no correlation concern
      Action: Validate
      File: specs/001-persistence-spi/contracts/repository.md
      Section: Behavioral Contract
      Outcome: Repository contract explicitly states correlation_id is not a concern
      Validation: `grep "not a Repository concern\|correlation_id-agnostic\|correlation_id" specs/001-persistence-spi/contracts/repository.md` returns at least 1 line

- [x] T009 Run quickstart Scenario 2 — Snapshot states no correlation concern
      Action: Validate
      File: specs/001-persistence-spi/contracts/snapshot.md
      Section: Behavioral Contract
      Outcome: Snapshot contract explicitly states correlation_id is not a concern
      Validation: `grep "not a Snapshot concern\|correlation_id-agnostic\|correlation_id" specs/001-persistence-spi/contracts/snapshot.md` returns at least 1 line

- [x] T010 Run quickstart Scenario 3 — EventStore states ownership
      Action: Validate
      File: specs/001-persistence-spi/contracts/event-store.md
      Section: Behavioral Contract
      Outcome: EventStore contract states ownership; Repository and Snapshot excluded
      Validation: `grep "exclusively owned\|owns\|owned" specs/001-persistence-spi/contracts/event-store.md | grep -i "correlation"` returns at least 1 line

- [x] T011 Run quickstart Scenario 4 — no behavioral changes
      Action: Validate
      File: workspace root
      Section: test suite
      Outcome: All existing tests pass
      Validation: `cargo test` — exits 0

---

## Dependencies & Execution Order

```
Phase 1 (Foundational) → Phase 2 (Scope Boundary) → Phase 3 (Validation)
```

### Parallel Opportunities

| Phase | [P] Tasks | Rationale |
|-------|-----------|-----------|
| Phase 1 | T001, T002, T003 | Different files, independent |
| Phase 2 | T005, T006, T007 | Different target files, independent |
| Phase 3 | T008, T009, T010, T011 | All independent validations |

### Execution Strategy

1. **T001–T003** in parallel
2. **T004** (create scope boundary section), then **T005, T006, T007** in parallel
3. **T008–T011** in parallel

---

## Definition of Done

- [x] `specs/001-persistence-spi/spec.md` has "Correlation Scope Boundary" subsection under Contract Invariants
- [x] `specs/001-persistence-spi/contracts/event-store.md` states correlation_id ownership
- [x] `specs/001-persistence-spi/contracts/repository.md` states correlation_id is not a concern
- [x] `specs/001-persistence-spi/contracts/snapshot.md` states correlation_id is not a concern
- [x] No code changes — `cargo test` passes
- [x] quickstart.md all scenarios pass
