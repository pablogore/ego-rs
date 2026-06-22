# Delta for Security SDK

## MODIFIED Requirements

### Requirement: NFR-005: No Ambient Security State

No code in `security-sdk` or `service-sdk` MUST store `SecurityContext` or `ServiceContext`
in a thread-local, task-local (`tokio::task_local!`), or global (`static`, `once_cell`,
`lazy_static`). The security field travels exclusively through explicit `ServiceContext`
passing. The service context itself MUST also travel exclusively through explicit parameter
passing — no task-local `CURRENT_CONTEXT` for `ServiceContext` is permitted.

(Previously: covered `SecurityContext` ambient storage only; this delta extends the prohibition
to `ServiceContext` task-local/thread-local/global patterns, aligning with CORE-010A.)

#### Scenario: No task-local or thread-local ServiceContext in codebase

- GIVEN the full workspace compiles successfully
- WHEN `grep -rn "task_local.*ServiceContext\|CURRENT_CONTEXT" crates/` is executed
- THEN zero matches are returned

#### Scenario: No task-local or thread-local SecurityContext in codebase

- GIVEN the full workspace compiles successfully
- WHEN `grep -rn "thread_local\|task_local\|lazy_static\|once_cell::sync::Lazy" crates/security-sdk/src/ crates/service-sdk/src/context/` is executed
- THEN zero matches related to security-context or service-context ambient storage are returned

#### Scenario: SecurityContext constructed without ambient side effects

- GIVEN a `Principal` value
- WHEN `SecurityContext::new(principal)` is called from two independent async tasks
- THEN neither task's context is visible from the other task
- AND no shared static or task-local storage is written

#### Scenario: ServiceContext not obtainable from ambient state

- GIVEN a component that needs a `ServiceContext`
- WHEN its source code is inspected
- THEN the `ServiceContext` value appears in at least one of: function parameter, constructor
  argument, or owned struct field — never in a `current()` call or task-local read
