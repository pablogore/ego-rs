# Delta for `production-composition-hardening`

> Canonical / English. Spanish companion: `spec.es.md` (1:1 requirement IDs and scenarios).
> The precise composition-visible signal that marks reservations as "required" for a given
> composition is a design-phase decision — see this spec's requirement below, which states the
> gate as an outcome (reject when required-and-non-durable, accept when durable or
> not-required) rather than naming the exact trigger.

## ADDED Requirements

### Requirement: Operation Reservation Store Gate Under Production

Under a production-grade profile, when a composition requires operation reservations for
correctness, that composition MUST be rejected if its registered `OperationReservationStore`
is not durable, and MUST be accepted if it is durable. When the composition does not require
operation reservations, no reservation store gate applies — a composition with no such
requirement is never forced to register a store it would never use. This mirrors the
conditional shape already used by the effect-store and read-side claim-store gates: the
capability is only governed once the composition's own configuration signals that it needs it.

#### Scenario: A non-durable reservation store is rejected when reservations are required

- GIVEN a production-grade profile and a composition that requires operation reservations for
  correctness
- WHEN the registered `OperationReservationStore` is not durable
- THEN the composition is rejected, naming the reservation store as the unmet capability and
  the exact registration call that fixes it

#### Scenario: A durable reservation store is accepted when reservations are required

- GIVEN a production-grade profile and a composition that requires operation reservations for
  correctness
- WHEN the registered `OperationReservationStore` is durable
- THEN the composition succeeds

#### Scenario: No gate applies when reservations are not required

- GIVEN a production-grade profile and a composition that does not require operation
  reservations for correctness
- WHEN the composition is validated
- THEN it succeeds regardless of whether a reservation store is registered, or of its
  durability

## MODIFIED Requirements

### Requirement: One Shared Predicate Is The Single Source Of Truth For The Rule

Exactly one shared predicate MUST decide "declared production + capability not durably
configured = refuse" for all five capabilities (event store, snapshot store, effect store,
read-side durable progress, operation reservation store). Because the capabilities live across
a one-way crate boundary (`persistent-entity` cannot see `service-sdk`'s effect-store,
read-side, or reservation types), this predicate cannot itself inspect either builder directly:
each composition surface (`EntityRuntimeBuilder`'s `validate_persistence()`, `RuntimeBuilder`'s
`validate_persistence_profile()`, including its read-side and reservation branches) MUST
compute its own capability's answer locally and pass it to the one shared predicate — never
restate the refuse/allow decision itself. No second, independently-maintained definition of the
decision MUST exist anywhere in the composition path.

(Previously: enumerated four capabilities — event store, snapshot store, effect store, and
read-side durable progress — without the operation reservation store as a fifth.)

#### Scenario: All three capabilities' decision routes through the same predicate

- GIVEN the composition path from `EntityRuntimeBuilder` and `RuntimeBuilder`/`AppBuilder`
- WHEN the codebase is inspected for capability-gating logic
- THEN every gate call site computes its own local answer (is a durable implementation
  configured for *this* capability?) and passes it to the one shared predicate that decides
  refuse-or-allow; no call site reimplements that decision itself, and no duplicate,
  independently drifting definition of "refuse" exists

#### Scenario: The fourth capability's decision routes through the same predicate

- GIVEN the read-side durable progress gate
- WHEN the codebase is inspected for its gating logic
- THEN it computes its own local answer (are both stores of a registered pair durable?) and
  passes it to the same shared predicate the other capabilities already use — no separate,
  independently-maintained read-side-only decision exists

#### Scenario: The fifth capability's decision routes through the same predicate

- GIVEN the operation reservation store gate added by this change
- WHEN the codebase is inspected for its gating logic
- THEN it computes its own local answer (is the registered `OperationReservationStore`
  durable, when one is required?) and passes it to the same shared predicate the other four
  capabilities already use — no separate, independently-maintained reservation-only decision
  exists

### Requirement: Rejections Are Actionable

Every rejection under this spec MUST name both the missing capability and the exact
configuration call that resolves it.

(Previously: enumerated four capabilities in its scenario; now includes the operation
reservation store as a fifth.)

#### Scenario: Error names the capability and the fix

- GIVEN any rejection produced by this spec's gate
- WHEN the error is inspected
- THEN it names the missing capability (event store, snapshot store, effect store, read-side
  durable progress, or operation reservation store) and the exact registration or builder call
  that configures it
