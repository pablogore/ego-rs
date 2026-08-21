# Spec: CORE-018 Production Reference Service

## Capability: reference-service (new)

Purpose: reference implementation of tenant-scoped user registration, exercising `PersistentEntity`, the service-sdk guard chain, and observability recording via existing public APIs only.

### Requirement: PersistentEntity Contracts for User and TenantOrganization

| Entity | Command | Event | State |
|---|---|---|---|
| `User` | register a user (identity + attributes) | `UserRegistered` | registered user record |
| `TenantOrganization` | ensure the org exists (idempotent) | `OrganizationEnsured` | `Absent` \| `Present{name}` |

Both MUST implement `PersistentEntity`'s Command/Event/State/`handle_command`/`apply_event` contract; state MUST reflect the last-applied event.

#### Scenario: Registering a user
- GIVEN no `User` entity exists for an identity
- WHEN the register command is handled
- THEN `UserRegistered` is produced and applied, and state reflects the registered user

#### Scenario: Ensuring a tenant org exists
- GIVEN a `TenantOrganization` entity in `Absent` state
- WHEN the `Ensure` command is handled
- THEN `OrganizationEnsured` is produced and applied, and state transitions to `Present{name}`
- AND WHEN `Ensure` is handled again on an already-`Present` org, no event is produced (idempotent)

### Requirement: RegisterUser Authorization and Tenant-Scoping

`RegisterUser` MUST be guarded by `#[authorize(permission="user:register")]` and `#[tenant_scoped]`; either guard denying the call MUST prevent any entity write.

#### Scenario: Unauthorized principal denied
- GIVEN a principal lacking `user:register` permission
- WHEN `RegisterUser` is invoked
- THEN the call is denied and no entity write occurs

#### Scenario: Cross-tenant request denied
- GIVEN an authorized principal whose `SecurityContext` tenant differs from the target tenant
- WHEN `RegisterUser` is invoked for that tenant
- THEN the call is denied and no entity write occurs

### Requirement: RegisterUser Happy Path

When both guards pass, `RegisterUser` MUST create the `User` entity and associate it with the target `TenantOrganization`, reporting success only when both writes complete.

#### Scenario: Successful registration
- GIVEN an authorized, tenant-scoped call with valid input
- WHEN the operation completes
- THEN a `User` entity exists and the `TenantOrganization` is `Present`

### Requirement: Non-Atomic Dual Write Is Documented, Not Hidden

`RegisterUser` MUST NOT use a saga or compensation mechanism. If the `TenantOrganization` write succeeds and the subsequent `User` write fails, `RegisterUser` MUST return an error, the `TenantOrganization` association MUST remain (no automatic rollback), and this outcome MUST be documented and observable — never a silent success or an unhandled panic.

#### Scenario: TenantOrganization succeeds, User write fails
- GIVEN an authorized, tenant-scoped call
- WHEN the `TenantOrganization` write succeeds but the `User` write fails
- THEN `RegisterUser` returns an error, and the `TenantOrganization` association persists without a matching `User` entity

### Requirement: RegisterUser Observability

Each invocation MUST record at least one `Observability` event: a success event when both writes complete, a failure event when a guard denies the call or the dual write partially fails.

#### Scenario: Success and failure are observed
- GIVEN a test-double `Observability` implementor
- WHEN `RegisterUser` succeeds, is denied, or partially fails
- THEN the test-double recorded a matching success or failure event for that call

### Requirement: Retried RegisterUser Produces Exactly One UserRegistered Event

A retried `POST /register` request carrying the identical `Idempotency-Key`
and identical request content MUST produce exactly one `UserRegistered` event
and exactly one welcome-email effect across all retries — closing the live
duplicate-registration defect in `UserEntity::handle_command`.

#### Scenario: Retry after a lost response does not duplicate registration
- GIVEN a `POST /register` request that succeeded but whose response was
  lost by the client
- WHEN the client retries with the identical `Idempotency-Key` and payload
- THEN exactly one `UserRegistered` event and one welcome-email effect exist
  for that operation — the retry observes the original outcome, not a
  second execution

### Requirement: Dual-Aggregate Recovery After Mid-Operation Process Death

`RegisterUserImpl` MUST recover correctly when the process dies after the
`TenantOrganization` write completes but before the `User` write is reached.
On retry (via lease takeover), the operation MUST complete with the
organization write treated as already-applied (no-op) and the user write
executed, producing zero duplicated events, per the explicit non-promise
that this recovery is not atomic across the two aggregates.

#### Scenario: Recovery completes without duplicating the organization write
- GIVEN a `RegisterUser(K)` operation whose lease expired after the
  `TenantOrganization` receipt was confirmed but before the `User` command
  executed
- WHEN a new owner takes over the lease and re-executes the operation
- THEN the `TenantOrganization` aggregate no-ops on its existing receipt, the
  `User` aggregate executes and receives its own receipt, and no
  `UserRegistered` event is duplicated

### Non-Goals
- No saga/compensation mechanism for the dual write.
- No production `Observability` adapter — test-double assertion only.
