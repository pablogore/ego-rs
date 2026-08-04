# Delta for HTTP Transport

## ADDED Requirements

### Requirement: Mandatory Idempotency-Key Extraction

The HTTP layer MUST extract an `Idempotency-Key` header on every mutable
route and reject the request before it reaches the guarded service operation
when the header is missing or fails newtype validation, under the default
(fail-closed) `IdempotencyEnforcementMode`.

#### Scenario: Missing key rejected before the operation runs
- GIVEN a mutable route under the default enforcement mode
- WHEN a request arrives with no `Idempotency-Key` header
- THEN the transport layer returns a rejection response and the guarded
  operation is never invoked

#### Scenario: Invalid key rejected before the operation runs
- GIVEN a mutable route under the default enforcement mode
- WHEN a request arrives with an `Idempotency-Key` header that fails
  `OperationKey` validation (e.g. empty after trim)
- THEN the transport layer returns a rejection response and the guarded
  operation is never invoked

#### Scenario: Valid key is carried into ServiceContext
- GIVEN a request with a valid `Idempotency-Key` header
- WHEN the request reaches the guarded operation
- THEN the operation's `ServiceContext` exposes the identical `OperationKey`

### Requirement: Replay and Conflict Responses Are Distinguishable

The HTTP layer MUST map a same-key-same-fingerprint replay to the original
successful response (within the reservation TTL), and a same-key-different-
fingerprint conflict to a distinguishable permanent-conflict response —
never a silent success and never identical to a fresh execution's response.

#### Scenario: Replay returns the original response, unexecuted
- GIVEN a completed operation under key K with fingerprint F, within its
  reservation TTL
- WHEN the same key K is retried with the identical fingerprint F
- THEN the HTTP layer returns the original stored response without
  re-invoking the operation

#### Scenario: Conflicting fingerprint returns a distinguishable error
- GIVEN a completed or in-progress operation under key K with fingerprint F
- WHEN key K is retried with a different fingerprint F'
- THEN the HTTP layer returns a response distinguishable from both a fresh
  success and a replay — a permanent conflict
