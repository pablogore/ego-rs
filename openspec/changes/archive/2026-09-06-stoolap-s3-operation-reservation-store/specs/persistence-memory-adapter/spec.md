# Delta for `persistence-memory-adapter`

> Canonical / English. Spanish companion: `spec.es.md` (1:1 requirement IDs and scenarios).

## ADDED Requirements

### Requirement: In-Memory Operation Reservation Store Reports Non-Durable

The in-memory `OperationReservationStore` implementation MUST report itself as non-durable:
its state does not survive a process restart, and it MUST NOT override the port's durability
signal to claim otherwise.

#### Scenario: The in-memory store reports non-durable

- GIVEN the in-memory `OperationReservationStore` implementation
- WHEN its durability is queried
- THEN it reports non-durable

#### Scenario: A production-profile composition still rejects it

- GIVEN a composition validated under a production-grade profile that registers the in-memory
  `OperationReservationStore` where a durable one is required
- WHEN the composition is validated
- THEN it is rejected, naming the reservation store as the unmet capability
