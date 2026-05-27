# Technical Quality Review

## SEVERITY: CRITICAL

### CRITICAL-01: Runtime slice not in workspace

**Problem:** `core/runtime-slice/` is a standalone crate, not a workspace member. No workspace crate can depend on it. The domain layer cannot reference runtime types. The application layer cannot integrate with the executor.

**Impact:** Framework kernel is unreachable. CORE-001 cannot progress until this is fixed.

**Fix:** Add `"core/runtime-slice"` to workspace members in root `Cargo.toml`. Rename package to `ego-runtime-slice` for naming consistency. Update `layers.toml` to assign it the domain layer (it defines types only).

---

### CRITICAL-02: Empty stubs masquerading as modules

**Problem:** 8 files in `core/runtime-slice/src/` are empty (executor, projection, validation, persistence, observability, main, example). They are not declared in `lib.rs`, so they are dead code. Governance dir was removed.

**Impact:** Misleading codebase state. Looks like work exists. Nothing is implemented.

**Fix:** Removed `main.rs`, `example.rs`, `governance/`. Remaining stubs must be implemented in CORE-001 or removed. No undeclared empty files.

---

## SEVERITY: HIGH

### HIGH-01: Actor spec is 342 lines of governance-tier bureaucracy

**Problem:** `specs/actor-model/spec.md` dedicates >40% of content to governance invariants, forbidden patterns, capability inflation protection, compliance verification, and enforcement mechanisms. The actual actor contract is ~60% of the document.

**Impact:** The spec is un-implementable as-is. No agent can process 57 governance requirements to implement `Actor::receive`.

**Fix:** Reduce to `Actor` trait contract, `ActorRef`, lifecycle states, message contract, supervision semantics. Governance moves to Phase 13. Done in this cleanup. Spec retained for atomic migration later.

---

### HIGH-02: Persistence SPI spec is 503 lines

**Problem:** 503 lines covering "durability semantics," "state persistence semantics," "event persistence semantics," "snapshot persistence semantics," "checked elimination proof," lifecycle model, capability model, constitutional invariants, compliance verification, and inflation protection. The SPI trait surface is ~50 lines.

**Impact:** Implementation paralysis. No agent can process 500 lines of ceremony to implement `EventStore::append`.

**Fix:** Reduce to: `EventStore` trait, `SnapshotStore` trait, replay semantics, deterministic guarantees. Remove governance tiers. Retained for future migration.

---

### HIGH-03: Contract tests are empty stubs

**Problem:** `crates/application/src/tests/contract_tests.rs` has two test functions with only `// Add test logic here` comments.

**Impact:** Zero integration testing. Hexagonal boundaries are untested.

**Fix:** Implement contract tests for `HelloHandler` against `CommandHandler` port. Test happy path, error path, type conformance.

---

## SEVERITY: MEDIUM

### MEDIUM-01: Application handler returns concrete error type

**Problem:** `HelloHandler::handle` returns `Result<String, HelloError>` instead of using the port's generic error. The port trait `CommandHandler<C>` returns `Result<(), Error>` but `HelloHandler` hardcodes `HelloError`.

**Impact:** Tight coupling between handler and port. Swapping handler implementations is harder.

**Fix:** Either make port error generic or define a common domain error for handlers. Not critical for MVP.

---

### MEDIUM-02: No mock runtime exists

**Problem:** Constitution mandates mock runtimes for tests. No mock runtime implementation exists.

**Impact:** Cannot test actor or runtime code constitutionally.

**Fix:** Build mock runtime as part of CORE-001 or CORE-002. In-memory, deterministic, time-controllable.

---

### MEDIUM-03: Runtime slice types bypass domain hexagon

**Problem:** `runtime-slice` crate defines `RuntimeSliceId`, `ExecutionContext`, `DeterministicInput` outside the domain layer. These are domain types living in an external crate.

**Impact:** Hexagonal architecture violation. Domain types should live in `ego-domain`, not `runtime-slice`.

**Fix:** When `runtime-slice` joins workspace, move domain types to `ego-domain` and have `runtime-slice` depend on `ego-domain`. Done as part of CORE-001.

---

## SEVERITY: LOW

### LOW-01: HelloQuery/HelloResponse are example boilerplate

**Problem:** Domain layer has `hello.rs` with `HelloQuery`/`HelloResponse`. Useful as a reference implementation but not framework code.

**Impact:** Minor noise. Serves as a working example of CQRS trait usage.

**Fix:** Keep as reference implementation. Don't expand. Remove when real domain types exist.

---

### LOW-02: transport/routes/ directory is empty

**Problem:** `crates/transport/src/routes/` directory exists but contains no files. Not declared in lib.rs.

**Impact:** Dead directory. No functional impact.

**Fix:** Remove empty directory or leave for CORE-008. Low priority. None.

---

## Summary

| Severity | Count | Status |
|----------|-------|--------|
| CRITICAL | 2 | 1 fixed (stubs removed), 1 pending (workspace integration) |
| HIGH | 3 | Specs simplified, contract tests pending |
| MEDIUM | 3 | Design decisions, not blockers |
| LOW | 2 | Cosmetic, non-blocking |