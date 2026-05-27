## ADDED Requirements

### Requirement: Governance routing

The runtime executor SHALL route every unit of work through `validate_runtime_invariants` before transitioning to `Running`. If governance rejects, the executor SHALL propagate the `ExecutionRejectionReason` as a fail-closed error and MUST NOT execute the work.

The executor integration target is:

- **File:** `core/runtime-slice/src/executor.rs`
- **Function call:** `ego_domain::governance::validate_runtime_invariants`

The executor SHALL expose `Result<T, ExecutionRejectionReason>` — no wrapping, no boxing, no type erasure.

#### Scenario: Governance called before execution
- **WHEN** a unit of work is submitted
- **THEN** the executor SHALL call `validate_runtime_invariants` before execution

#### Scenario: Governance rejection is fail-closed
- **WHEN** governance rejects the work
- **THEN** the executor SHALL fail closed — reject, propagate reason, do not execute

#### Scenario: Governance module path frozen
- **WHEN** the executor calls governance
- **THEN** it SHALL import from `ego_domain::governance` (crate `ego-domain`, path `crates/domain/src/governance/`)

#### Scenario: Error type is ExecutionRejectionReason
- **WHEN** the executor returns an error from governance
- **THEN** the error type SHALL be `ExecutionRejectionReason` — not `anyhow::Error`, not `Box<dyn Error>`, not `String`