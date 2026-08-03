# Delta for Persistent Entity

## ADDED Requirements

### Requirement: Receipt Consultation Gates Dispatch and Recovery

Before dispatching a command carrying an `operation_key` to a `PersistentEntity`
handler, the actor MUST consult that aggregate's persisted receipt for
`(tenant_id, aggregate_type, aggregate_id, operation_key)`. If a receipt exists
with a matching fingerprint, the actor MUST no-op (return the receipt's
recorded outcome) rather than re-invoking `handle_command`. If a receipt
exists with a different fingerprint, the actor MUST return a permanent
conflict and MUST NOT invoke `handle_command`.

#### Scenario: Already-applied operation no-ops instead of re-executing
- GIVEN a receipt exists for `(tenant, User, user-7, K)` with fingerprint F
- WHEN a command carrying key K and fingerprint F is dispatched to the actor
  for `user-7`
- THEN `handle_command` is never invoked; the actor returns the receipt's
  recorded outcome

#### Scenario: Fingerprint mismatch is a permanent conflict, not a re-execution
- GIVEN a receipt exists for `(tenant, User, user-7, K)` with fingerprint F
- WHEN a command carrying key K and a different fingerprint F' is dispatched
- THEN the actor returns a permanent conflict; `handle_command` is not invoked

### Requirement: Zero-Event Branch Opens a Transaction to Confirm a Receipt

The actor's zero-event success branch (today: `CommandResult::NoEvents`,
never opening a transaction) MUST open a transaction to durably confirm the
operation's receipt for that aggregate, even though no event is appended.

#### Scenario: A zero-event success still produces a durable receipt
- GIVEN a command whose `handle_command` returns no events (e.g. an
  already-idempotent domain-level "Ensure")
- WHEN the actor completes the command
- THEN a transaction opens and confirms the receipt for that aggregate and
  operation key, where previously no transaction was opened at all

### Requirement: CommandContext Carries the Operation Key

`CommandContext` MUST carry the `OperationKey` established at ingress through
to the actor and its receipt-consultation/confirmation logic.

#### Scenario: Operation key reaches the actor unchanged
- GIVEN an `OperationKey` established at HTTP ingress for a command
- WHEN the command reaches `EntityActor::execute_command` via
  `CommandContext`
- THEN the identical `OperationKey` value is available for receipt lookup
  and confirmation

### Requirement: Aggregate Identity Is Structurally Distinct, Not Concatenated

`EntityTriple::aggregate_id()` (or its replacement) MUST expose
`aggregate_type` and `aggregate_id` as distinct identity components rather
than producing a single concatenated string (e.g. via a hyphen join) for
persistence. Two different `(aggregate_type, aggregate_id)` pairs that would
collide under the previous concatenation scheme MUST resolve to distinct
persisted streams.

#### Scenario: Previously-colliding pairs no longer collide
- GIVEN aggregate type `user-account` with id `7`, and aggregate type `user`
  with id `account-7`
- WHEN both are persisted through `EntityTriple`
- THEN they resolve to two distinct persisted streams, not one shared string
