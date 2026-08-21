# Delta for Reference Service

## ADDED Requirements

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
