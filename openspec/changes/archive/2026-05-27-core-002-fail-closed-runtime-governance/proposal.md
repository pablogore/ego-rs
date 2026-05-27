## Why

The runtime-abstraction spec requires fail-closed behavior and deterministic execution, but provides no enforcement. Invariant violations and undefined states can proceed silently. CORE-002 introduces `validate_runtime_invariants` — a free function in the domain layer that rejects execution before it begins whenever invariants cannot be proven.

## What Changes

- Add `governance` submodule at `crates/domain/src/governance/` (in `ego-domain` crate) containing:
  - `lifecycle_state.rs` — `LifecycleState` enum (Pending, Running, Completed, Failed, Cancelled, TimedOut)
  - `governance_context.rs` — `GovernanceContext` struct (`slice_id: String`, `inputs_present: bool`)
  - `execution_rejection_reason.rs` — `ExecutionRejectionReason` enum (`InvalidTransition`, `UndefinedState`)
  - `validation.rs` — `transition_is_valid` and `validate_runtime_invariants` free functions
  - `mod.rs` — re-exports all 5 public items
- Add `pub mod governance;` to `crates/domain/src/lib.rs`
- Add `ego-domain` dependency to `core/runtime-slice/Cargo.toml`
- Wire governance call into `core/runtime-slice/src/executor.rs` — call `ego_domain::governance::validate_runtime_invariants` before execution
- Freeze: canonical Display output, canonical UndefinedState messages, validation precedence order, derive style, executor error propagation contract
- Remove any execution path that bypasses governance

## Capabilities

### New Capabilities
- None — governance is a single module, not a constitutional capability.

### Modified Capabilities
- `runtime-abstraction`: Runtime MUST route through governance before execution at `core/runtime-slice/src/executor.rs`. Violations propagate as fail-closed `ExecutionRejectionReason`.

## Impact

- New domain governance module: `crates/domain/src/governance/` — 5 files, free functions, two enums, one minimal context struct
- Modified executor boundary at `core/runtime-slice/src/executor.rs` — call `validate_runtime_invariants` before execution
- Modified `core/runtime-slice/Cargo.toml` — add `ego-domain` dependency
- Tests: all governance paths tested with real function, no mocks for governance itself
- No changes to transport, infrastructure, or application layer