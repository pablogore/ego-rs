# Ambient Context Detection Audit

**Change**: CORE-010A post-hardening pass
**Date**: 2026-06-22
**Scope**: All Rust source files under `crates/`

---

## Audit Commands and Results

| Pattern | Command | Result |
|---------|---------|--------|
| ServiceContext::current() | `rg "ServiceContext::current" crates/ --type rust` | ZERO MATCHES ✅ |
| ServiceContext::scope() | `rg "ServiceContext::scope" crates/ --type rust` | ZERO MATCHES ✅ |
| CURRENT_CONTEXT task-local | `rg "CURRENT_CONTEXT" crates/ --type rust` | ZERO MATCHES ✅ |
| task_local! | `rg "task_local!" crates/ --type rust` | ZERO MATCHES ✅ |
| thread_local! | `rg "thread_local!" crates/ --type rust` | ZERO MATCHES ✅ |
| OnceCell | `rg "OnceCell" crates/ --type rust` | ZERO MATCHES ✅ |
| LazyLock | `rg "LazyLock" crates/ --type rust` | 1 match — EXEMPT (see below) ✅ |

---

## Approved Ambient-Like Constructs

The following constructs are **explicitly approved** because they do not participate in
execution-context propagation. Future additions require architecture review.

| Location | Construct | Purpose | Approved |
|----------|-----------|---------|---------|
| `crates/domain/src/actor.rs` — `actor_id!` macro | `LazyLock<ActorId>` | `ActorId` string interning | ✅ 2026-06-22 |

### Detail: LazyLock in `crates/domain/src/actor.rs`

**Location**: `crates/domain/src/actor.rs:153-154`

**Pattern**:
```rust
static ID: ::std::sync::LazyLock<$crate::actor::ActorId> =
    ::std::sync::LazyLock::new(|| { ... });
```

**Context**: Inside the `actor_id!` macro. Interns a static `&ActorId` reference to avoid
repeated heap allocation of the same actor identity string. The `LazyLock` holds an `ActorId`,
not any form of execution context.

**Verdict**: APPROVED — unrelated to context propagation. Does not violate NFR-002 or INV-001.

---

## Compliance Matrix

| Requirement | Check | Status |
|-------------|-------|--------|
| FR-001: No ambient ServiceContext | `rg "ServiceContext::current\|ServiceContext::scope\|CURRENT_CONTEXT" crates/` → 0 matches | ✅ PASS |
| FR-002: Explicit params only | All test files pass with `cargo test --workspace`; method signatures verified | ✅ PASS |
| FR-003: No thread/task-local/singleton for ServiceContext | All ambient pattern greps return 0 ServiceContext matches | ✅ PASS |
| FR-004: Propagation unchanged | Proxy codegen tests pass; interceptor order tests pass | ✅ PASS |

---

## Post-Hardening Actions Applied

1. **TASK-001/002**: Broader ambient prohibition added to `openspec/specs/service-sdk/spec.md`
2. **TASK-003**: Spawned task invariant (INV-004) added to spec
3. **TASK-004**: API contract documentation added to spec and COOKBOOK.md
4. **TASK-005**: Clone semantics documented on `ServiceContext` struct
5. **TASK-006**: `context_scope.rs` renamed to `context_explicit_propagation.rs`
6. **TASK-007**: This audit report committed
