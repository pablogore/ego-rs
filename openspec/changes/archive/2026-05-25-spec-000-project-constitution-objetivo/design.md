## Context

The project already has governance specs for architecture and testing. SPEC-000 sits above those specs as the project constitution: the stable rule set that future changes must respect. This change should not duplicate existing governance details. It should define the constitution, link the governance specs conceptually, and establish how constitution changes are made.

## Goals / Non-Goals

**Goals:**
- Create a `project-constitution` OpenSpec capability.
- Capture the immutable project rules as testable SHALL/MUST requirements.
- Define the constitution amendment rule so changes remain append-only and explicit.
- Make future OpenSpec proposals accountable to constitution requirements.

**Non-Goals:**
- Refactor existing Rust crates.
- Rebuild CI enforcement already covered by governance changes.
- Replace `architecture-governance` or `testing-governance`.
- Create an exception/waiver system.

## Decisions

### Constitution as a Dedicated Capability

SPEC-000 is represented as `project-constitution` rather than as edits to `architecture-governance` or `testing-governance`. This keeps project-level principles separate from enforcement details and avoids recreating specs that already exist.

### Existing Governance Specs Remain Enforcement Standards

The constitution references architecture and testing governance as mandatory standards. Detailed layer rules, mock requirements, and coverage enforcement remain in their dedicated specs.

### Amendments Require OpenSpec Changes

The constitution is immutable by default. Any modification requires a new OpenSpec change that states the reason, migration impact, and compatibility strategy. This preserves append-only lineage and prevents hidden rule changes.

### No Waivers

The first version of SPEC-000 does not define waivers. If a rule becomes impractical, the correct path is a constitution amendment, not an undocumented exception.

## Risks / Trade-offs

- [Risk] The constitution may feel rigid early in the project -> Mitigation: amendments are allowed, but they must be explicit OpenSpec changes.
- [Risk] Future changes may forget to check SPEC-000 -> Mitigation: add tasks for README documentation and contributor checklist updates.
- [Risk] Existing code may not comply immediately -> Mitigation: this change defines the rule set; compliance implementation can happen through follow-up changes.
- [Risk] Rules overlap with governance specs -> Mitigation: SPEC-000 states principles and delegates detailed enforcement to dedicated governance specs.
