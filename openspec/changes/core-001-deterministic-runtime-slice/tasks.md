## 1. Workspace Integration

- [x] 1.1 Add `core/runtime-slice` to workspace members in root `Cargo.toml`
- [ ] 1.2 Rename package to `ego-runtime-slice` for naming consistency (optional, match workspace convention)
- [x] 1.3 Verify `cargo build --workspace` succeeds with runtime-slice as member

## 2. Executor Implementation

- [x] 2.1 Implement `crate::executor` in `core/runtime-slice/src/executor.rs`
- [x] 2.2 Executor SHALL accept units of work and transition them through lifecycle states (Pending → Running → Completed/Failed)
- [x] 2.3 Executor SHALL fail closed on ambiguous states
- [x] 2.4 Executor SHALL produce deterministic observable semantics

## 3. Projection Implementation

- [x] 3.1 Implement `crate::projection` in `core/runtime-slice/src/projection.rs`
- [x] 3.2 Projection SHALL materialize execution outcomes into observable semantics
- [x] 3.3 Observable semantics SHALL be non-mutating

## 4. Validation Implementation

- [x] 4.1 Implement `crate::validation` in `core/runtime-slice/src/validation.rs`
- [x] 4.2 Validate deterministic equivalence across multiple executions with identical inputs
- [x] 4.3 Validate replay equivalence (original vs replay produces same semantics)
- [x] 4.4 Validate fail-closed behavior on ambiguous states

## 5. Module Wiring

- [x] 5.1 Declare `executor`, `projection`, `validation` modules in `core/runtime-slice/src/lib.rs`
- [x] 5.2 Re-export public API types

## 6. Tests

- [x] 6.1 Test: executor runs unit of work deterministically — identical inputs → identical observable semantics
- [x] 6.2 Test: executor transitions work Pending → Running → Completed/Failed
- [x] 6.3 Test: executor fail-closed on ambiguous state — rejects, does not execute
- [x] 6.4 Test: projection materializes non-mutating observable semantics
- [x] 6.5 Test: replay produces identical observable semantics as original execution
- [x] 6.6 Test: validation confirms deterministic equivalence
- [x] 6.7 Test: runtime slice is infrastructure-free — no I/O, no network, no database

## 7. Verification

- [x] 7.1 Run `cargo test --workspace` — all tests pass
- [x] 7.2 Run `cargo clippy --workspace -- -D warnings` — no warnings
- [x] 7.3 Verify runtime-slice is a workspace member
