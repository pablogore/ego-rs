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

### Non-Goals
- No saga/compensation mechanism for the dual write.
- No production `Observability` adapter — test-double assertion only.
