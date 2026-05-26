## Why

The project needs a formal constitution that defines immutable rules for architecture, development process, testing, lineage, observability, and compatibility. Without a single normative spec, these rules drift into informal convention and become hard to enforce consistently.

## What Changes

- Introduce SPEC-000 as the Project Constitution.
- Define constitution-level requirements for deterministic-first behavior, fail-closed decisions, explicit state, append-only lineage, OpenSpec-driven development, mandatory hexagonal architecture, CQRS + event-driven design, >=95% test coverage, no real resources in unit tests, observability by default, and backward compatibility.
- Establish that future changes must comply with the constitution or explicitly propose a constitution amendment.
- Reference existing `architecture-governance` and `testing-governance` specs as enforcement standards rather than recreating them.

## Capabilities

### New Capabilities

- `project-constitution`: Immutable project rules and amendment process for all future specs and implementations.

### Modified Capabilities

<!-- None - existing governance specs are referenced by the constitution, not modified by this change. -->

## Impact

- All future OpenSpec changes must be reviewed against SPEC-000.
- Contributors get one canonical source for project-level rules.
- Existing architecture and testing governance remain active standards under the constitution.
- Future CI and review tooling can validate that changes acknowledge constitution requirements.
