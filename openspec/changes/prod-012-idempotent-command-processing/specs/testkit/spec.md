# Delta for TestKit

## ADDED Requirements

### Requirement: Reservation-Store Test Double

TestKit MUST provide an `OperationReservationStore` double that satisfies the
identical port `service-sdk` registers in production, letting a test
deterministically control reservation outcomes (`Fresh`, `OwnedInProgress`,
`OtherInProgress`, `Succeeded`, `Conflict`, `StaleOwner`) and lease
expiry/takeover without a real durable backend.

#### Scenario: Test configures a deterministic lease expiry
- GIVEN a TestKit reservation-store double wired with a deterministic test
  `Clock`
- WHEN the test advances the `Clock` past a configured lease's `lease_until`
- THEN a subsequent operation against that reservation observes it as
  eligible for takeover, deterministically and without a real backend

#### Scenario: Double satisfies the same port production code depends on
- GIVEN a service under test that depends on `OperationReservationStore`
- WHEN it is supplied the TestKit double
- THEN the service runs unmodified, exercising the identical port contract
  production code depends on
