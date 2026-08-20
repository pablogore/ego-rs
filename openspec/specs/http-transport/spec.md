# Spec: CORE-018 Production Reference Service

## Capability: http-transport (new)

Purpose: a real axum-based HTTP server in `ego-transport` exposing `RegisterUser` over a network socket.

### Requirement: HTTP Route Reaches RegisterUser

The system MUST expose an HTTP route (e.g. `POST /register`) that routes a request through `Runtime::resolve` to the `RegisterUser` service operation and returns its outcome to the caller.

#### Scenario: Request reaches the guarded operation
- GIVEN a running axum server with the reference-app runtime mounted
- WHEN a client sends a request with valid credentials and payload to the route
- THEN `RegisterUser` is invoked via `Runtime::resolve` and the response reflects its outcome

### Requirement: Security Context Extraction From Requests

Each incoming request MUST carry authenticable credentials that the transport layer maps to a `SecurityContext` before invoking `RegisterUser`; requests without valid credentials MUST NOT reach the guarded operation. (Exact extraction mechanism, e.g. JWT `Authorization` header vs. simpler principal injection, is left to design.md — OQ-2.)

#### Scenario: Missing or invalid credentials rejected pre-invocation
- GIVEN a request lacking valid authenticable credentials
- WHEN it reaches the route
- THEN the transport layer returns an error response and `RegisterUser` is never invoked

#### Scenario: Valid credentials produce a SecurityContext
- GIVEN a request with valid authenticable credentials
- WHEN it reaches the route
- THEN a `SecurityContext` is derived and passed into the `RegisterUser` invocation

### Requirement: Success/Error Response Contract

The HTTP layer MUST map a successful `RegisterUser` outcome to a success response, and each documented failure outcome (authorization denial, tenant-scoping denial, partial dual-write failure) to a distinguishable error response, without exposing internal diagnostic detail.

#### Scenario: Outcomes map to appropriate responses
- GIVEN `RegisterUser` succeeds, is denied, or partially fails
- WHEN the HTTP layer builds a response
- THEN the response category matches the outcome, and no raw internal error/entity detail is exposed

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

### Non-Goals
- No gRPC transport, no general-purpose transport framework.
- No admin UI.
- No multi-region/clustering concerns.
