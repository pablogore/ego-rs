# Gap Analysis — Framework Capabilities

## MISSING

### Foundational

| Capability | Status | Priority |
|-----------|--------|----------|
| **Runtime kernel** | Partially exists in `core/runtime-slice`. Has types but no executor, no workspace membership. | CRITICAL |
| **Actor primitive** | Spec exists (core-002), zero code. `Actor` trait, `ActorRef`, `ActorSystem::spawn`. | CRITICAL |
| **Message dispatch** | Domain traits exist (Command, Event, Query) but no dispatch infrastructure. `CommandBus`, `EventBus`, `QueryBus`. | HIGH |
| **Mailbox model** | Not specified. Per-actor mailbox, ordering, bounded capacity. | HIGH |
| **Supervision** | Spec exists in actor model but zero code. Parent-child, restart/stop/escalate. | HIGH |

### Implementation

| Capability | Status | Priority |
|-----------|--------|----------|
| **Persistence SPI** | Spec exists (core-003), zero code. Event sourcing + snapshot port. | HIGH |
| **Observability SPI** | Spec exists (core-004), zero code. Tracing/metrics/logging port. | MEDIUM |
| **Transport** | `crates/transport/` is doc-comment-only. gRPC/HTTP handlers needed. | MEDIUM |
| **SDK + Developer API** | Nothing. Derive macros, `#[actor]` proc macro, config builder. | LOW |
| **Examples** | Nothing. Empty `core/runtime-slice/src/example.rs` removed. | LOW |

### Integration

| Capability | Status | Priority |
|-----------|--------|----------|
| **Workspace integration** | `core/runtime-slice/` is NOT a workspace member. Cannot be used by any crate. | CRITICAL |
| **Contract tests** | `application/src/tests/contract_tests.rs` has empty test stubs. | MEDIUM |
| **CI pipeline** | Exists in `.github/workflows/`. Needs actual test targets. | MEDIUM |

### Testing

| Capability | Status | Priority |
|-----------|--------|----------|
| **Mock runtime** | No mock runtime exists for testing actor-dependent code. | HIGH |
| **In-memory persistence adapter** | No test adapter exists. | HIGH |
| **Integration test framework** | Nothing beyond the empty contract_test stubs. | MEDIUM |

---

## UNNECESSARY (Archived)

| Removed Capability | Why |
|-------------------|-----|
| Constitutional ownership chain (9-layer model) | Zero code, pure bureaucracy, enterprise theater |
| Capability inflation protection (governance tier) | Premature — framework doesn't exist yet |
| Compliance verification mechanisms | Premature — nothing to verify compliance of |
| Governance violation detection | Premature — governance before runtime |
| Semantic loop correction | Spec-ception — fixing a spec that has no code |
| Examples constitution | Meta-governance — govern examples after writing them |
| Determism constitution (separate document) | Merged into project constitution |
| Dependency governance constitution | Already enforced by layers.toml |
| Canonical contracts constitution | Already covered by archive/foundation-002 |

---

## DUPLICATED (Resolved)

| Duplicate | Resolution |
|-----------|-----------|
| `changes/foundation-003` (active) = `archive/foundation-003` = `specs/runtime-abstraction` | Active copy archived. Canonical spec simplified. |
| `foundation-005-persistence-spi` vs `foundation-017-persistence-model` | SPI kept (core-003), model archived with chain. |
| `foundation-006-cluster-model` vs `foundation-018-placement-model` | Cluster kept (deferred), placement archived. |
| `foundation-019-lifecycle-model` vs lifecycle in `runtime-abstraction` + `actor-model` | Lifecycle model archived. Lifecycle belongs in specific domain specs. |

---

## MISPLACED

| Issue | Resolution |
|-------|-----------|
| `core/runtime-slice/` not in workspace | Must join workspace. Blocked until Cargo.toml updated. |
| Governance in `crates/domain/` with zero code | Removed. Re-created in Phase 13 when runtime exists. |
| Empty transport/infrastructure crates | Will fill in CORE-007/CORE-004. Not blocked. |