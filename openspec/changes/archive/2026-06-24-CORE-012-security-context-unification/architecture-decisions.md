# Architecture Decisions — CORE-012 Security Context Unification

These decisions are inputs to `design.md`. Incorporated during proposal phase via Q1–Q6 and subsequent clarifications.

## AD-001 SecurityContext Ownership

`security-sdk` is the sole owner of `SecurityContext`. Domain must not define a `SecurityContext` type.

## AD-002 Claims Lifetime

Claims are request-scoped. Claims MUST NOT be persisted in:
- aggregates
- events
- snapshots
- projections
- repositories

## AD-003 Principal Scope

`Principal` represents identity only.

Allowed:
- subject
- tenant_id
- roles

Forbidden:
- permissions
- authorization decisions
- policy cache

## AD-004 Synchronous Authentication Contract

`AuthenticationProvider` is synchronous: `fn authenticate(...)`. Future key acquisition must occur before authentication execution. Authentication itself performs no I/O.

## AD-005 Explicit Security Propagation

`SecurityContext` may only be propagated explicitly. Forbidden: `thread_local!`, `task_local!`, `OnceCell`, `LazyLock`, global state. Extends CORE-009A.

## AD-006 Authorization Independence

`AuthorizationProvider` must not depend on raw JWT claims. Authorization decisions must be derived from `Principal`, `Role`, and `Permission` only. Claims are transport metadata and must not be inspected inside authorization logic.

## AD-007 ServiceContext Security Propagation

`ServiceContext.security: Option<SecurityContext>` is the exclusive propagation mechanism for authenticated context at the runtime level. All access to authenticated identity and authorization decisions flows through `ServiceContext`. The field is additive — `None` by default preserves backward compatibility. Extends AD-005.

## AD-008 Principal Claim Ownership

`Principal` MUST NOT contain `claims`. All raw authentication claims live exclusively in `SecurityContext.claims`.

Rationale:
- avoids duplication (Principal.claims and SecurityContext.claims would overlap)
- preserves AD-003 identity-only Principal
- preserves AD-006 authorization independence (authZ reads Principal and Roles, not raw claims)
