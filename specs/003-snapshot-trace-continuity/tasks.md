# Tasks: Snapshot Trace Continuity

**Input**: [spec.md](spec.md), [plan.md](plan.md)

**Prerequisites**: `plan.md` (required), `spec.md` (required), `research.md` (completed), `quickstart.md` (completed)

**MVP Scope**: Update Persistence SPI contract documentation (spec 001) with explicit snapshot trace continuity guarantees. No code changes, no Snapshot trait signature changes.

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

- [x] T001 [P] Verify spec 001 spec.md identifies Snapshot contract sections
      Action: Validate
      File: specs/001-persistence-spi/spec.md
      Section: Contract Invariants, Key Entities, Requirements
      Outcome: Snapshot sections located; Snapshot entity description found; location for trace continuity invariants identified
      Validation: `grep -n "Snapshot\|snapshot" specs/001-persistence-spi/spec.md | head -5` returns lines

- [x] T002 [P] Verify spec 001 snapshot contract exists
      Action: Validate
      File: specs/001-persistence-spi/contracts/snapshot.md
      Section: Trait Contract, Behavioral Contract
      Outcome: Snapshot contract file found; trait signature confirmed without correlation_id
      Validation: `grep "correlation_id" specs/001-persistence-spi/contracts/snapshot.md` exits 1 (no matches)

---

## Phase 2: Trace Continuity Documentation (US1, US3)

**Purpose**: Add trace continuity invariants documenting that snapshot restore + delta replay preserves correlation_ids. Satisfies FR-001, FR-004, FR-005.

- [x] T003 [US1] Add trace continuity invariants to spec 001 spec.md
      Action: Modify
      File: specs/001-persistence-spi/spec.md
      Section: Contract Invariants — new "Snapshot Trace Continuity" subsection
      Outcome: New "Snapshot Trace Continuity" subsection with invariants: (1) snapshot restore + delta replay preserves correlation_ids, (2) trace equivalence between snapshot+replay and full replay, (3) delta events carry original correlation_ids unchanged. Snapshot entity description updated to note correlation_id is out of scope.
      Validation: `grep -c "Trace Continuity\|trace continuity" specs/001-persistence-spi/spec.md` returns at least 1

- [x] T004 [US3] Add trace equivalence invariant
      Action: Modify
      File: specs/001-persistence-spi/spec.md
      Section: Contract Invariants — Snapshot Trace Continuity
      Outcome: Trace equivalence invariant: snapshot restore + delta replay produces correlation_id chain identical to full stream replay for the overlapping version range.
      Validation: `grep "trace equival\|identical to\|overlapping" specs/001-persistence-spi/spec.md` returns at least 1 line

- [x] T005 [US1] Update snapshot contract with trace continuity guarantee
      Action: Modify
      File: specs/001-persistence-spi/contracts/snapshot.md
      Section: Behavioral Contract — Critical Invariants
      Outcome: New invariant: "Snapshot restore + EventStore delta replay preserves event correlation_ids. Snapshot itself does not carry correlation_id — trace continuity is maintained by the EventStore."
      Validation: `grep "trace continuity\|delta replay\|EventStore" specs/001-persistence-spi/contracts/snapshot.md` returns at least 1 line

---

## Phase 3: Snapshot Optionality Documentation (US2)

**Purpose**: Document that Snapshot MAY omit correlation_id. Satisfies FR-003.

- [x] T006 [US2] Document snapshot optionality in spec 001 spec.md
      Action: Modify
      File: specs/001-persistence-spi/spec.md
      Section: Contract Invariants — Snapshot Trace Continuity
      Outcome: Explicit statement: "Snapshot SHALL NOT define, store, or require correlation_id. Correlation_id is owned by the EventStore. Snapshot is correlation_id-free."
      Validation: `grep "SHALL NOT\|correlation_id-free\|EventStore" specs/001-persistence-spi/spec.md | grep -i "snapshot"` returns at least 1 line

- [x] T007 [US2] Document snapshot optionality in snapshot contract
      Action: Modify
      File: specs/001-persistence-spi/contracts/snapshot.md
      Section: Behavioral Contract — Critical Invariants
      Outcome: Explicit statement: "Snapshot does not carry correlation_id. Optional by design — correlation_id is an Event-only concept."
      Validation: `grep "does not carry\|correlation_id-free\|not.*concern\|Event-only" specs/001-persistence-spi/contracts/snapshot.md` returns at least 1 line

---

## Phase 4: Quickstart Validation

**Purpose**: Execute validation scenarios from quickstart.md.

- [x] T008 Run quickstart Scenario 1 — snapshot contract does not carry correlation_id
      Action: Validate
      File: specs/001-persistence-spi/contracts/snapshot.md
      Section: Trait Contract
      Outcome: Snapshot trait signature confirmed without correlation_id parameter
      Validation: `grep "correlation_id" specs/001-persistence-spi/contracts/snapshot.md` exits 1 (no matches)

- [x] T009 Run quickstart Scenario 2 — trace continuity documented in spec 001
      Action: Validate
      File: specs/001-persistence-spi/spec.md
      Section: Contract Invariants — Snapshot Trace Continuity
      Outcome: Trace continuity section present with all invariants
      Validation: `grep -c "Trace Continuity\|trace continuity" specs/001-persistence-spi/spec.md` returns at least 1

- [x] T010 Run quickstart Scenario 4 — no behavioral changes
      Action: Validate
      File: workspace root
      Section: test suite
      Outcome: All existing tests pass
      Validation: `cargo test` — exits 0

---

## Dependencies & Execution Order

```
Phase 1 (Foundational) → Phase 2 (Trace Continuity) → Phase 3 (Optionality) → Phase 4 (Validation)
```

### Parallel Opportunities

| Phase | [P] Tasks | Rationale |
|-------|-----------|-----------|
| Phase 1 | T001, T002 | Different files |
| Phase 3 | T006, T007 | Different target files |

### Execution Strategy

1. **T001–T002** in parallel
2. **T003** (create trace continuity section), then **T004** (add equivalence invariant)
3. **T005** in parallel with T003–T004 (different file)
4. **T006, T007** in parallel
5. **T008–T010** in parallel

---

## Definition of Done

- [x] `specs/001-persistence-spi/spec.md` has "Snapshot Trace Continuity" subsection under Contract Invariants
- [x] Trace continuity invariant: snapshot restore + delta replay preserves correlation_ids
- [x] Trace equivalence invariant: snapshot+replay matches full replay for overlapping range
- [x] Snapshot optionality documented: Snapshot does not carry correlation_id
- [x] `specs/001-persistence-spi/contracts/snapshot.md` updated with trace continuity guarantee
- [x] No code changes — `cargo test` passes
- [x] quickstart.md all scenarios pass
