## 1. Module Setup

- [x] 1.1 Create `crates/domain/src/governance/` directory
- [x] 1.2 Create `crates/domain/src/governance/lifecycle_state.rs`
- [ ] 1.3 Create `crates/domain/src/governance/governance_context.rs`
- [ ] 1.4 Create `crates/domain/src/governance/execution_rejection_reason.rs`
- [ ] 1.5 Create `crates/domain/src/governance/validation.rs`
- [ ] 1.6 Create `crates/domain/src/governance/mod.rs` — declare submodules and re-export: `LifecycleState`, `GovernanceContext`, `ExecutionRejectionReason`, `transition_is_valid`, `validate_runtime_invariants`
- [ ] 1.7 Add `pub mod governance;` to `crates/domain/src/lib.rs`

## 2. Domain Types

- [ ] 2.1 Add `LifecycleState` enum in `lifecycle_state.rs` — variants `Pending`, `Running`, `Completed`, `Failed`, `Cancelled`, `TimedOut` — `#[derive(Debug, Clone, PartialEq, Eq)]`, manual `impl Display` with canonical strings: `"Pending"`, `"Running"`, `"Completed"`, `"Failed"`, `"Cancelled"`, `"TimedOut"`
- [ ] 2.2 Add `GovernanceContext` struct in `governance_context.rs` — fields `slice_id: String`, `inputs_present: bool` — `#[derive(Debug, Clone, PartialEq, Eq)]`
- [ ] 2.3 Add `ExecutionRejectionReason` enum in `execution_rejection_reason.rs` — two variants only: `InvalidTransition { current: LifecycleState, requested: LifecycleState }`, `UndefinedState(&'static str)` — `#[derive(Debug, Clone, PartialEq, Eq)]`, manual `impl Display` with canonical format: `InvalidTransition(current=<current>, requested=<requested>)` and `UndefinedState(<message>)`. Do NOT implement `Error` trait.

## 3. Governance Logic

- [ ] 3.1 Implement `transition_is_valid` free function in `validation.rs` — `Pending→Running` and `Running→terminal` are valid; no transitions from terminal states
- [ ] 3.2 Implement `validate_runtime_invariants` free function in `validation.rs` — calls checks in frozen order:
  - 1. `transition_is_valid(current, requested)` → if false, return `Err(InvalidTransition { current, requested })`
  - 2. `context.slice_id` non-empty → if empty, return `Err(UndefinedState("slice_id is empty"))`
  - 3. `context.inputs_present == true` → if false, return `Err(UndefinedState("inputs are not present"))`
  - 4. Return `Ok(())`
- [ ] 3.3 Ensure both functions are deterministic — no I/O, no wall-clock, no randomness

## 4. Executor Integration

- [ ] 4.1 Add `ego-domain = { path = "../../crates/domain" }` dependency to `core/runtime-slice/Cargo.toml`
- [ ] 4.2 In `core/runtime-slice/src/executor.rs`, call `ego_domain::governance::validate_runtime_invariants` before transitioning work to `Running`
- [ ] 4.3 Executor boundary SHALL expose `Result<T, ExecutionRejectionReason>` — no `anyhow`, no `Box<dyn Error>`, no `String`, no panic
- [ ] 4.4 On governance rejection, propagate `ExecutionRejectionReason` unchanged — do NOT execute, do NOT transition to `Running`

## 5. Tests — Transition Validation

- [ ] 5.1 Test: `transition_is_valid(Pending, Running)` returns `true`
- [ ] 5.2 Test: `transition_is_valid(Running, Completed)` returns `true`
- [ ] 5.3 Test: `transition_is_valid(Completed, Running)` returns `false`
- [ ] 5.4 Test: `transition_is_valid(Failed, Running)` returns `false`
- [ ] 5.5 Test: `transition_is_valid(Cancelled, Running)` returns `false`
- [ ] 5.6 Test: `transition_is_valid(TimedOut, Running)` returns `false`
- [ ] 5.7 Test: `transition_is_valid(Running, Pending)` returns `false`
- [ ] 5.8 Test: `transition_is_valid` is deterministic — same inputs twice returns same result

## 6. Tests — Invariant Validation (Happy Path)

- [ ] 6.1 Test: `validate_runtime_invariants` returns `Ok(())` for valid transition (Pending→Running) with `slice_id = "valid"` and `inputs_present = true`

## 7. Tests — Invariant Validation (Invalid Transition)

- [ ] 7.1 Test: `validate_runtime_invariants` returns `Err(InvalidTransition { current: Completed, requested: Running })` for Completed→Running (with valid context)

## 8. Tests — Invariant Validation (Undefined State)

- [ ] 8.1 Test: `validate_runtime_invariants` returns `Err(UndefinedState("slice_id is empty"))` for empty `slice_id` (with valid transition Pending→Running and `inputs_present = true`)
- [ ] 8.2 Test: `validate_runtime_invariants` returns `Err(UndefinedState("inputs are not present"))` for `inputs_present = false` (with valid transition Pending→Running and non-empty `slice_id`)

## 9. Tests — Precedence Ordering (First Failure Wins)

- [ ] 9.1 Precedence test: `current = Completed`, `requested = Running`, `slice_id = ""`, `inputs_present = true` → returns `Err(InvalidTransition { current: Completed, requested: Running })` — NOT `UndefinedState("slice_id is empty")` — because transition validation runs first
- [ ] 9.2 Precedence test: `current = Pending`, `requested = Running`, `slice_id = ""`, `inputs_present = false` → returns `Err(UndefinedState("slice_id is empty"))` — NOT `UndefinedState("inputs are not present")` — because slice_id check runs second
- [ ] 9.3 Precedence test: `current = Pending`, `requested = Running`, `slice_id = "valid"`, `inputs_present = false` → returns `Err(UndefinedState("inputs are not present"))` — inputs_present is third

## 10. Tests — Canonical Display Output

- [ ] 10.1 Test: `LifecycleState::Pending` Display → exact string `"Pending"`
- [ ] 10.2 Test: `LifecycleState::Running` Display → exact string `"Running"`
- [ ] 10.3 Test: `LifecycleState::Completed` Display → exact string `"Completed"`
- [ ] 10.4 Test: `LifecycleState::Failed` Display → exact string `"Failed"`
- [ ] 10.5 Test: `LifecycleState::Cancelled` Display → exact string `"Cancelled"`
- [ ] 10.6 Test: `LifecycleState::TimedOut` Display → exact string `"TimedOut"`
- [ ] 10.7 Test: `ExecutionRejectionReason::InvalidTransition { current: Completed, requested: Running }` Display → exact string `"InvalidTransition(current=Completed, requested=Running)"`
- [ ] 10.8 Test: `ExecutionRejectionReason::UndefinedState("slice_id is empty")` Display → exact string `"UndefinedState(slice_id is empty)"`
- [ ] 10.9 Test: `ExecutionRejectionReason::UndefinedState("inputs are not present")` Display → exact string `"UndefinedState(inputs are not present)"`

## 11. Tests — Canonical UndefinedState Messages

- [ ] 11.1 Test: rejecting empty `slice_id` produces `UndefinedState("slice_id is empty")` — exact message equality
- [ ] 11.2 Test: rejecting `inputs_present: false` produces `UndefinedState("inputs are not present")` — exact message equality

## 12. Tests — Determinism

- [ ] 12.1 Test: `validate_runtime_invariants` returns identical result for identical inputs — call twice, assert deep equality

## 13. Tests — Executor Integration

- [ ] 13.1 Test: executor calls `validate_runtime_invariants` before executing work
- [ ] 13.2 Test: executor rejects and propagates `ExecutionRejectionReason` when governance fails — without wrapping in `anyhow`, `Box<dyn Error>`, or `String`
- [ ] 13.3 Test: executor does NOT transition to `Running` after governance rejection
- [ ] 13.4 Test: executor returns `Result<T, ExecutionRejectionReason>` — verify the concrete error type, not a trait object

## 14. Tests — Derive Behavior Verification

All three domain types SHALL derive `#[derive(Debug, Clone, PartialEq, Eq)]`. Verification tests confirm trait behavior (not implementation provenance).

- [ ] 14.1 Test: `LifecycleState` Clone and PartialEq — `LifecycleState::Pending.clone() == LifecycleState::Pending` is `true`, and `Pending != Running`
- [ ] 14.2 Test: `GovernanceContext` Clone and PartialEq — clone a context, assert equality; change a field, assert inequality
- [ ] 14.3 Test: `ExecutionRejectionReason` Clone and PartialEq — clone an `InvalidTransition`, assert equality; clone an `UndefinedState`, assert equality

## 15. Verification

- [ ] 15.1 Run `cargo test --workspace` — all tests pass
- [ ] 15.2 Run `cargo clippy --workspace -- -D warnings` — no warnings
- [ ] 15.3 Verify `core/runtime-slice/Cargo.toml` contains `ego-domain` dependency
- [ ] 15.4 Verify `crates/domain/src/lib.rs` contains `pub mod governance;`
- [ ] 15.5 Verify `crates/domain/src/governance/mod.rs` re-exports all 5 public items
- [ ] 15.6 Verify by code review: `ExecutionRejectionReason` does NOT implement `std::error::Error` — no `impl Error for ExecutionRejectionReason`, no `#[derive(Error)]`. Verification through implementation inspection and clippy, NOT through compile-fail test harness.
