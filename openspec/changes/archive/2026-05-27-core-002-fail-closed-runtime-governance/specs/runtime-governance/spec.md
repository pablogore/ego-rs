## ADDED Requirements

### Requirement: Module Ownership

Governance SHALL live in `crates/domain/src/governance/` within the `ego-domain` workspace crate.

Files SHALL be:

```
crates/domain/src/governance/
├── lifecycle_state.rs
├── governance_context.rs
├── execution_rejection_reason.rs
├── validation.rs
└── mod.rs
```

`mod.rs` SHALL declare and re-export:

```rust
pub mod lifecycle_state;
pub mod governance_context;
pub mod execution_rejection_reason;
pub mod validation;

pub use lifecycle_state::LifecycleState;
pub use governance_context::GovernanceContext;
pub use execution_rejection_reason::ExecutionRejectionReason;
pub use validation::{transition_is_valid, validate_runtime_invariants};
```

#### Scenario: Module declared in domain crate
- **WHEN** the domain crate is compiled
- **THEN** `ego_domain::governance` SHALL contain `LifecycleState`, `GovernanceContext`, `ExecutionRejectionReason`, `transition_is_valid`, `validate_runtime_invariants`

---

### Requirement: Derive Style

`Debug`, `Clone`, `PartialEq`, and `Eq` implementations SHALL use Rust derive macros.

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
```

Manual implementations are forbidden unless `derive` is impossible (e.g., `Display`).

#### Scenario: LifecycleState uses derive
- **WHEN** `LifecycleState` is defined
- **THEN** `Debug`, `Clone`, `PartialEq`, `Eq` SHALL be derived via `#[derive(...)]`, not manually implemented

#### Scenario: GovernanceContext uses derive
- **WHEN** `GovernanceContext` is defined
- **THEN** `Debug`, `Clone`, `PartialEq`, `Eq` SHALL be derived via `#[derive(...)]`, not manually implemented

#### Scenario: ExecutionRejectionReason uses derive
- **WHEN** `ExecutionRejectionReason` is defined
- **THEN** `Debug`, `Clone`, `PartialEq`, `Eq` SHALL be derived via `#[derive(...)]`, not manually implemented

---

### Requirement: Governance failure denies execution

If governance cannot prove execution validity, execution SHALL be denied. Execution SHALL fail closed. Execution MUST NOT continue. The executor SHALL propagate the rejection.

#### Scenario: Invalid execution denied
- **WHEN** governance rejects execution
- **THEN** execution SHALL NOT proceed

#### Scenario: Undefined state denied
- **WHEN** governance cannot prove execution validity
- **THEN** execution SHALL fail closed

#### Scenario: Rejection propagated
- **WHEN** governance rejects execution
- **THEN** the rejection SHALL be propagated to the caller

---

### Requirement: LifecycleState

A `LifecycleState` enum SHALL exist in the domain governance module with the following variants: `Pending`, `Running`, `Completed`, `Failed`, `Cancelled`, `TimedOut`. It SHALL implement `Display`, `Debug`, `Clone`, `PartialEq`, `Eq`. `Debug`, `Clone`, `PartialEq`, `Eq` SHALL use `#[derive(...)]`.

`Display` output SHALL be deterministic and canonical:

| Variant    | Display string |
|-----------|---------------|
| `Pending`   | `"Pending"`    |
| `Running`   | `"Running"`    |
| `Completed` | `"Completed"`  |
| `Failed`    | `"Failed"`     |
| `Cancelled` | `"Cancelled"`  |
| `TimedOut`  | `"TimedOut"`   |

No lowercase. No localization. No formatting variation. No free-form formatting.

#### Scenario: Canonical Display for Pending
- **WHEN** `LifecycleState::Pending` is displayed
- **THEN** the output SHALL be the exact string `"Pending"`

#### Scenario: Canonical Display for Running
- **WHEN** `LifecycleState::Running` is displayed
- **THEN** the output SHALL be the exact string `"Running"`

#### Scenario: Canonical Display for Completed
- **WHEN** `LifecycleState::Completed` is displayed
- **THEN** the output SHALL be the exact string `"Completed"`

#### Scenario: Canonical Display for Failed
- **WHEN** `LifecycleState::Failed` is displayed
- **THEN** the output SHALL be the exact string `"Failed"`

#### Scenario: Canonical Display for Cancelled
- **WHEN** `LifecycleState::Cancelled` is displayed
- **THEN** the output SHALL be the exact string `"Cancelled"`

#### Scenario: Canonical Display for TimedOut
- **WHEN** `LifecycleState::TimedOut` is displayed
- **THEN** the output SHALL be the exact string `"TimedOut"`

---

### Requirement: GovernanceContext

A `GovernanceContext` struct SHALL exist in the domain governance module with the following fields:
- `slice_id: String`
- `inputs_present: bool`

It SHALL implement `Debug`, `Clone`, `PartialEq`, `Eq`. All four SHALL use `#[derive(...)]`. No speculative fields. No runtime adapter types.

---

### Requirement: transition_is_valid

A free function `transition_is_valid` SHALL exist in the domain governance module:

```rust
pub fn transition_is_valid(
    current: &LifecycleState,
    requested: &LifecycleState,
) -> bool
```

It SHALL return `true` for:
- `Pending` → `Running`
- `Running` → `Completed`, `Failed`, `Cancelled`, `TimedOut`

It SHALL return `false` for:
- Any transition from `Completed`, `Failed`, `Cancelled`, or `TimedOut`
- Any transition not listed as allowed

No trait. No dyn dispatch. No callback.

#### Scenario: Valid transition from Pending
- **WHEN** `transition_is_valid(Pending, Running)` is called
- **THEN** it SHALL return `true`

#### Scenario: Valid transition from Running
- **WHEN** `transition_is_valid(Running, Completed)` is called
- **THEN** it SHALL return `true`

#### Scenario: Invalid transition from terminal
- **WHEN** `transition_is_valid(Completed, Running)` is called
- **THEN** it SHALL return `false`

#### Scenario: Deterministic result
- **WHEN** `transition_is_valid` is called twice with identical arguments
- **THEN** it SHALL return the identical result both times

---

### Requirement: ExecutionRejectionReason

An `ExecutionRejectionReason` enum SHALL exist in the domain governance module with the following variants only:
- `InvalidTransition { current: LifecycleState, requested: LifecycleState }`
- `UndefinedState(&'static str)`

The enum SHALL implement `Display`, `Debug`, `Clone`, `PartialEq`, `Eq`. `Debug`, `Clone`, `PartialEq`, `Eq` SHALL use `#[derive(...)]`. It SHALL NOT implement `Error` or `std::error::Error`.

`Display` output SHALL be deterministic and canonical:

`InvalidTransition { current, requested }` SHALL display as:

```
InvalidTransition(current=<current>, requested=<requested>)
```

where `<current>` and `<requested>` are the canonical `Display` output of the respective `LifecycleState` variants.

Example: `InvalidTransition(current=Completed, requested=Running)`

`UndefinedState(message)` SHALL display as:

```
UndefinedState(<message>)
```

Example: `UndefinedState(slice_id is empty)`

No lowercase. No formatting variation. No free-form formatting.

#### Scenario: InvalidTransition rejection
- **WHEN** governance rejects a transition from `Running` to `Pending`
- **THEN** the rejection SHALL be `ExecutionRejectionReason::InvalidTransition` with `current: Running` and `requested: Pending`

#### Scenario: UndefinedState rejection
- **WHEN** `GovernanceContext` has `inputs_present: false` or `slice_id` is empty
- **THEN** the rejection SHALL be `ExecutionRejectionReason::UndefinedState` with a description of what is undefined

#### Scenario: Canonical Display for InvalidTransition
- **WHEN** `ExecutionRejectionReason::InvalidTransition { current: LifecycleState::Completed, requested: LifecycleState::Running }` is displayed
- **THEN** output SHALL be the exact string `"InvalidTransition(current=Completed, requested=Running)"`

#### Scenario: Canonical Display for UndefinedState
- **WHEN** `ExecutionRejectionReason::UndefinedState("slice_id is empty")` is displayed
- **THEN** output SHALL be the exact string `"UndefinedState(slice_id is empty)"`

---

### Requirement: Canonical UndefinedState Messages

`UndefinedState(&'static str)` SHALL accept only these canonical messages:

- `"slice_id is empty"`
- `"inputs are not present"`

No synonyms. No alternative wording. No speculative messages. No other string constant is permitted.

#### Scenario: Empty slice_id message
- **WHEN** `validate_runtime_invariants` rejects due to empty `slice_id`
- **THEN** the rejection SHALL be `ExecutionRejectionReason::UndefinedState("slice_id is empty")`

#### Scenario: Missing inputs message
- **WHEN** `validate_runtime_invariants` rejects due to `inputs_present: false`
- **THEN** the rejection SHALL be `ExecutionRejectionReason::UndefinedState("inputs are not present")`

---

### Requirement: validate_runtime_invariants

A free function `validate_runtime_invariants` SHALL exist in the domain governance module with the following signature:

```rust
pub fn validate_runtime_invariants(
    context: &GovernanceContext,
    current: &LifecycleState,
    requested: &LifecycleState,
) -> Result<(), ExecutionRejectionReason>
```

#### Validation Order

Validation SHALL run in frozen order. First failure wins. Subsequent checks SHALL NOT execute after a rejection.

Order:

1. `transition_is_valid(current, requested)` — if `false`, return `Err(InvalidTransition { current, requested })`
2. `context.slice_id` non-empty — if empty, return `Err(UndefinedState("slice_id is empty"))`
3. `context.inputs_present == true` — if `false`, return `Err(UndefinedState("inputs are not present"))`
4. If all checks pass, return `Ok(())`

The function SHALL be deterministic — identical inputs always produce identical output. The function SHALL NOT perform I/O, access wall-clock time, or depend on external state.

#### Scenario: All invariants pass
- **WHEN** `validate_runtime_invariants` is called with valid transition and defined context
- **THEN** it SHALL return `Ok(())`

#### Scenario: Invalid transition rejected
- **WHEN** `validate_runtime_invariants` is called with an invalid transition
- **THEN** it SHALL return `Err(ExecutionRejectionReason::InvalidTransition)`

#### Scenario: Undefined context rejected
- **WHEN** `validate_runtime_invariants` is called with `inputs_present: false`
- **THEN** it SHALL return `Err(ExecutionRejectionReason::UndefinedState)`

#### Scenario: Deterministic behavior
- **WHEN** `validate_runtime_invariants` is called twice with identical inputs
- **THEN** it SHALL return the identical result both times

#### Scenario: Precedence — transition validation runs first
- **WHEN** `current = Completed`, `requested = Running`, and `slice_id = ""`
- **THEN** `validate_runtime_invariants` SHALL return `Err(ExecutionRejectionReason::InvalidTransition { current: Completed, requested: Running })` — NOT `UndefinedState("slice_id is empty")` — because transition validation runs first

#### Scenario: Precedence — slice_id runs second
- **WHEN** `current = Pending`, `requested = Running`, `slice_id = ""`, and `inputs_present = false`
- **THEN** `validate_runtime_invariants` SHALL return `Err(ExecutionRejectionReason::UndefinedState("slice_id is empty"))` — NOT `UndefinedState("inputs are not present")` — because slice_id check runs second

#### Scenario: Precedence — inputs_present runs third
- **WHEN** `current = Pending`, `requested = Running`, `slice_id = "valid-id"`, and `inputs_present = false`
- **THEN** `validate_runtime_invariants` SHALL return `Err(ExecutionRejectionReason::UndefinedState("inputs are not present"))`

---

### Requirement: Executor Error Propagation Contract

The executor boundary SHALL expose operations returning:

```rust
Result<T, ExecutionRejectionReason>
```

The executor SHALL NOT:
- Wrap `ExecutionRejectionReason` into `anyhow::Error`
- Wrap `ExecutionRejectionReason` into `Box<dyn Error>`
- Convert `ExecutionRejectionReason` to `String`
- Panic on governance rejection
- Erase the rejection type

Governance rejection SHALL propagate as `ExecutionRejectionReason` unchanged. No adapters. No boxing. No translation layer.

#### Scenario: Executor returns Result<T, ExecutionRejectionReason>
- **WHEN** the executor boundary exposes an operation
- **THEN** its error type SHALL be `ExecutionRejectionReason`, not `anyhow::Error`, not `Box<dyn Error>`, not `String`

#### Scenario: Governance rejection propagates unchanged
- **WHEN** `validate_runtime_invariants` returns `Err(ExecutionRejectionReason::InvalidTransition { current: Pending, requested: Completed })`
- **THEN** the executor SHALL propagate that exact `ExecutionRejectionReason` value unchanged to the caller

---

### Requirement: Executor Integration Target

Governance SHALL be wired into the executor at:

- **File:** `core/runtime-slice/src/executor.rs`
- **Crate:** `runtime-slice` (standalone crate under `core/`)

For `runtime-slice` to call governance functions from `ego-domain`, its `Cargo.toml` SHALL add:

```toml
[dependencies]
ego-domain = { path = "../../crates/domain" }
```

The executor module SHALL call `ego_domain::governance::validate_runtime_invariants` before transitioning any work to `Running`.

#### Scenario: Executor depends on ego-domain
- **WHEN** `core/runtime-slice/Cargo.toml` is inspected
- **THEN** it SHALL contain `ego-domain` as a dependency

#### Scenario: Executor calls validate_runtime_invariants
- **WHEN** a unit of work is submitted to `core/runtime-slice/src/executor.rs`
- **THEN** the executor SHALL call `ego_domain::governance::validate_runtime_invariants` before executing

---

### Requirement: Governance enforcement at execution boundary

The executor SHALL call `validate_runtime_invariants` before executing any unit of work. If validation returns `Err`, the executor SHALL NOT transition the work to `Running` and SHALL propagate the rejection reason through the error channel. The executor MUST NOT bypass governance.

#### Scenario: Work validated before execution
- **WHEN** a unit of work is submitted to the executor
- **THEN** the executor SHALL call `validate_runtime_invariants` before transitioning the work to `Running`

#### Scenario: Governance rejection stops execution
- **WHEN** `validate_runtime_invariants` returns `Err(reason)`
- **THEN** the executor SHALL NOT execute the work and SHALL NOT transition to `Running`

#### Scenario: Governance rejection propagated
- **WHEN** `validate_runtime_invariants` returns `Err(reason)`
- **THEN** the executor SHALL propagate `reason` through the error channel

#### Scenario: Bypass is forbidden
- **WHEN** the executor executes work without calling `validate_runtime_invariants`
- **THEN** this SHALL be treated as a governance violation

#### Scenario: Executor does NOT wrap rejection
- **WHEN** governance rejects execution
- **THEN** the executor SHALL propagate `ExecutionRejectionReason` unchanged — no `anyhow`, no `Box<dyn Error>`, no `String`, no panic