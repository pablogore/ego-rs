# Delta for `persistence-api-surface`

> Canonical / English. Spanish companion: `spec.es.md` (1:1 requirement IDs and scenarios).

## ADDED Requirements

### Requirement: OperationReservationStore Durability Signal

`OperationReservationStore` MUST expose a way for an implementation to report whether it
durably persists reservations across a process restart. The default answer, for any
implementation that does not override it, MUST be the conservative one — non-durable — so a
third-party or not-yet-considered implementation is never mis-classified as durable. Wrapping
an implementation in a shared-ownership handle (e.g. `Arc`) MUST NOT change what durability it
reports: the wrapper MUST forward the inner implementation's answer, never fall back to the
default.

#### Scenario: A bare implementation that does not override the signal reports non-durable

- GIVEN an `OperationReservationStore` implementation that does not override the durability
  signal
- WHEN its durability is queried
- THEN it reports non-durable

#### Scenario: An implementation that overrides the signal reports what it declares

- GIVEN an `OperationReservationStore` implementation that overrides the durability signal to
  report durable
- WHEN its durability is queried
- THEN it reports durable

#### Scenario: A shared-ownership wrapper forwards the inner answer, durable case

- GIVEN an `OperationReservationStore` implementation that reports durable
- WHEN it is placed behind a shared-ownership handle and the handle's durability is queried
- THEN the handle reports durable — the same answer the inner implementation reports, not the
  trait's default

#### Scenario: A shared-ownership wrapper forwards the inner answer, non-durable case

- GIVEN an `OperationReservationStore` implementation that reports non-durable
- WHEN it is placed behind a shared-ownership handle and the handle's durability is queried
- THEN the handle reports non-durable
