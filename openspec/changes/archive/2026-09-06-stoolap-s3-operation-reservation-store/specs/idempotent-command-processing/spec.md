# Delta for `idempotent-command-processing`

> Canonical / English. Spanish companion: `spec.es.md` (1:1 requirement IDs and scenarios).

## ADDED Requirements

### Requirement: PostgreSQL-Backed Reservation Store Reports Durable

The PostgreSQL-backed `OperationReservationStore` MUST report itself as durable: it already
satisfies cross-restart persistence with atomic ownership transfer, so declaring anything other
than durable would understate a guarantee it already provides.

#### Scenario: The PostgreSQL-backed store reports durable

- GIVEN the PostgreSQL-backed `OperationReservationStore`
- WHEN its durability is queried
- THEN it reports durable
