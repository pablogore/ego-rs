## Context

The runtime-abstraction spec defines a Determinism Axiom and execution lifecycle (Pending → Running → ... → terminal). These are spec-level statements with no enforcement. CORE-002 introduces `validate_runtime_invariants` — a free function in the domain governance module — that the executor calls before every execution. Governance is not an SPI, not a pluggable capability, not configurable. It is a single function that checks concrete invariants against domain-owned types and returns a typed rejection or acceptance.

## Goals / Non-Goals

**Goals:**
- `validate_runtime_invariants` is a free function, no trait dispatch
- `transition_is_valid` is a free function — no trait, no dyn, no callback
- Rejection returns `InvalidTransition` or `UndefinedState` — no zombie variants
- Governance is synchronous — call before execute, no deferred validation
- Governance depends only on domain-owned `GovernanceContext` — no adapter types
- Governance is deterministic — identical inputs produce identical output
- Executor rejects before transitioning to Running if governance fails

**Non-Goals:**
- No `RuntimeInvariants` trait — transition validation is a free function
- No `GovernanceViolation` or `DeterminismViolation` catch-all variants
- No `RuntimeState` struct — governance takes concrete domain types
- No `Error` trait on `ExecutionRejectionReason` — Display + Debug + PartialEq only
- No mock for governance in tests — governance IS a pure function, test it directly
- No orchestration, policy DSL, plugin system, or enterprise architecture

## Decisions

### 1. Module Location and Ownership

Governance lives in `crates/domain/src/governance/` within the `ego-domain` workspace crate. The `ego-domain` crate depends on nothing internally (hexagonal domain layer).

File layout:

```
crates/domain/src/governance/
├── lifecycle_state.rs             # LifecycleState enum
├── governance_context.rs          # GovernanceContext struct
├── execution_rejection_reason.rs  # ExecutionRejectionReason enum
├── validation.rs                  # transition_is_valid, validate_runtime_invariants
└── mod.rs                         # Re-exports
```

`crates/domain/src/lib.rs` declares:

```rust
pub mod governance;
```

`mod.rs` exports:

```rust
pub use lifecycle_state::LifecycleState;
pub use governance_context::GovernanceContext;
pub use execution_rejection_reason::ExecutionRejectionReason;
pub use validation::{transition_is_valid, validate_runtime_invariants};
```

No invented folders. No speculative layering. Governance is a domain submodule.

### 2. Free function, not trait + impl

Governance is constitutional and non-pluggable. There is exactly one implementation. A free function `validate_runtime_invariants` is the minimal primitive — no vtable, no dispatch, no abstraction overhead.

```rust
pub fn validate_runtime_invariants(
    context: &GovernanceContext,
    current: &LifecycleState,
    requested: &LifecycleState,
) -> Result<(), ExecutionRejectionReason>
```

### 3. transition_is_valid as free function, not trait

Transition validation is deterministic and constitutional. A free function replaces the `RuntimeInvariants` trait — no trait, no dyn dispatch, no runtime ownership.

```rust
pub fn transition_is_valid(
    current: &LifecycleState,
    requested: &LifecycleState,
) -> bool
```

Allowed transitions:
- `Pending` → `Running`
- `Running` → `Completed | Failed | Cancelled | TimedOut`
- Terminal states (`Completed`, `Failed`, `Cancelled`, `TimedOut`) — no outgoing transitions

### 4. GovernanceContext — domain-owned, not adapter-owned

Governance must not depend on `ExecutionContext` from `runtime-slice` (hexagonal boundary violation). A minimal `GovernanceContext` is defined in the domain governance module:

```rust
pub struct GovernanceContext {
    pub slice_id: String,
    pub inputs_present: bool,
}
```

No speculative fields. No business logic. No runtime ownership.

### 5. LifecycleState enum — defined in domain governance module

The lifecycle states (Pending, Running, Completed, Failed, Cancelled, TimedOut) are defined in the runtime-abstraction spec. This change introduces them as a concrete Rust enum in the domain governance module so governance can reference them.

### 6. Typed rejection, no Error trait

`ExecutionRejectionReason` is a domain enum, not an error. It implements `Display`, `Debug`, `Clone`, `PartialEq`, `Eq`. Two variants only:

- `InvalidTransition { current, requested }` — which transition was invalid
- `UndefinedState(&'static str)` — what is undefined

No `DeterminismViolation`. No `GovernanceViolation`. No zombie variants.

### 7. Synchronous, before transition to Running

Governance runs before any state mutation. The executor calls `validate_runtime_invariants` with the current state and requested transition. If governance rejects, the executor MUST NOT transition to Running and MUST propagate the rejection reason.

### 8. Governance failure denies execution — constitutional centerpiece

If governance cannot prove execution validity, execution is denied. This is not best-effort validation. Governance is the gate. Failure to validate SHALL produce rejection, never continuation.

### 9. Canonical Display — deterministic output

`LifecycleState` Display SHALL be frozen:

| Variant      | Display string  |
|-------------|----------------|
| `Pending`     | `"Pending"`      |
| `Running`     | `"Running"`      |
| `Completed`   | `"Completed"`    |
| `Failed`      | `"Failed"`       |
| `Cancelled`   | `"Cancelled"`    |
| `TimedOut`    | `"TimedOut"`     |

No lowercase. No localization. No free-form formatting.

`ExecutionRejectionReason` Display SHALL be frozen:

```
InvalidTransition(current=<state>, requested=<state>)
UndefinedState(<message>)
```

where `<state>` is the canonical `LifecycleState` Display output.

Example: `InvalidTransition(current=Completed, requested=Running)`
Example: `UndefinedState(slice_id is empty)`

### 10. Canonical UndefinedState Messages — frozen strings

`UndefinedState(&'static str)` accepts only two strings:

- `"slice_id is empty"`
- `"inputs are not present"`

No synonyms. No alternative wording. No other string constant is permitted.

### 11. Validation Order and Precedence — frozen

First failure wins. Order is constitutional and frozen:

1. `transition_is_valid(current, requested)` — if false, return `InvalidTransition`
2. `context.slice_id` non-empty — if empty, return `UndefinedState("slice_id is empty")`
3. `context.inputs_present == true` — if false, return `UndefinedState("inputs are not present")`

Example of precedence: if `transition_is_valid` fails AND `slice_id` is empty, the rejection is `InvalidTransition`, NOT `UndefinedState`. Transition validity is the constitutional gate and runs first.

### 12. Executor Error Propagation Contract

The executor boundary SHALL expose `Result<T, ExecutionRejectionReason>`.

The executor MUST NOT:
- Wrap into `anyhow::Error`
- Wrap into `Box<dyn Error>`
- Convert to `String`
- Panic
- Erase the rejection type

Governance rejection propagates as `ExecutionRejectionReason` unchanged. No adapters. No boxing. No translation layer.

### 13. Executor Integration Target — frozen

Governance is wired into:

- **File:** `core/runtime-slice/src/executor.rs`
- **Crate:** `runtime-slice` (standalone crate under `core/`)

For `runtime-slice` to call `ego-domain`, its `Cargo.toml` SHALL add:

```toml
[dependencies]
ego-domain = { path = "../../crates/domain" }
```

The executor module calls `ego_domain::governance::validate_runtime_invariants` before transitioning any work to `Running`.

### 14. Derive Style — mandatory for Debug, Clone, PartialEq, Eq

All three domain types SHALL use `#[derive(Debug, Clone, PartialEq, Eq)]`. Manual implementations are forbidden. `Display` may be implemented manually (required for deterministic output).

Verification tests confirm trait behavior (Clone, PartialEq, Eq equality/inequality). They do NOT attempt to prove implementation provenance — that is a code-review and `#[derive]` attribute concern, not a runtime test concern.

### 15. Dependency Direction — hexagonal safety

The dependency graph SHALL be:

```
core/runtime-slice  ───►  crates/domain (ego-domain)
```

and NOT the reverse.

`ego-domain` currently depends on nothing internally (serde, thiserror, chrono — all external). Adding governance types to `ego-domain` does not introduce any internal dependency. `runtime-slice` acquiring a dependency on `ego-domain` is a one-directional edge from a non-workspace crate into the domain layer — consistent with hexagonal architecture.

No circular dependency is introduced. No architectural violation.

## Architecture

```
 Execution submitted
         │
         ▼
 ┌───────────────────────────────┐
 │ validate_runtime_invariants() │
 │  ─ transition_is_valid?       │
 │  ─ slice_id non-empty?        │
 │  ─ inputs_present?            │
 └───────────┬───────────────────┘
             │
     valid   │   invalid
     ┌───────┘   ┌──────────────────┐
     ▼           ▼                  │
 ┌────────┐ ┌──────────┐           │
 │Execute  │ │Deny      │           │
 │work     │ │propagate │           │
 │         │ │reason    │           │
 └────────┘ └──────────┘           │
                                    │
     Denial is NEVER silent ────────┘
```

## Risks / Trade-offs

- **[Governance gap]** If validation is too narrow, invalid states pass → Governance is constitutional; gaps are constitutional violations, fixed via spec change
- **[Performance]** Synchronous call on every execution → O(1) checks only — no I/O, no iteration, no allocation
- **[Bypass]** Executor skips governance → Fix the executor, not the governance layer. Governance is a function call, not a security boundary.