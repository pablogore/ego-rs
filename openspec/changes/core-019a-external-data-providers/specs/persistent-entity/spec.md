# Delta for persistent-entity

**Capability choice**: modifies the existing `persistent-entity` capability
(`openspec/specs/persistent-entity/spec.md`). Per proposal.md §10, this
delta was originally deferred as CONDITIONAL on which resolution model
design (AD-003) picked. `design.md` has now resolved AD-003 as a hybrid:
the handler-facing facade port and its DTOs (reached by a handler to fetch
external data through the CORE-019A provider mechanism) are owned by
`persistent-entity` itself (AD-001, AD-009) — not one design alternative
among several but the chosen shape — so this capability modification is
unconditional as of design completion, and this delta is no longer
speculative.

This document is written at the same **observable-contract level** as the
sibling `external-data-providers` capability
(`openspec/changes/core-019a-external-data-providers/specs/external-data-providers/spec.md`):
it does not restate that capability's resolution model, fetch signature,
registry key shape, or error taxonomy — those requirements, and their
concrete shape, belong to `external-data-providers` and `design.md`. This
delta only fixes what `persistent-entity` itself must guarantee to a
handler that uses the capability it exposes.

## ADDED Requirements

### Requirement: Handler-Reachable External Data Access

A `PersistentEntity` handler MUST be able to obtain external data during
command handling through a capability that `persistent-entity` exposes to
it, without depending on runtime-internal types or constructing an
external client inline. That capability is backed by whichever provider
the surrounding application has registered for a given key (see
`external-data-providers`); `persistent-entity` is not the registration
owner and does not implement provider logic — it exposes the
handler-reachable surface and obtains its backing from the runtime.

#### Scenario: Handler fetches external data during command handling

- GIVEN a handler's command-handling code needs data from a registered
  external data provider
- WHEN it invokes the fetch capability `persistent-entity` exposes to it
- THEN it receives the provider's response without depending on any
  runtime-internal type or constructing an external client inline

### Requirement: Missing Registration Fails Closed From the Handler's Perspective

When a handler fetches external data for a key with no registered
provider, `persistent-entity`'s exposed fetch capability MUST surface that
failure to the handler explicitly — never a silent default, empty value,
or no-op result. (Registration and resolution semantics themselves are
`external-data-providers`'s fail-closed resolution requirement; this
requirement only fixes that the failure is observable through the
persistent-entity-owned surface a handler actually uses.)

#### Scenario: Handler observes an explicit error for an unregistered key

- GIVEN no provider is registered for key `K`
- WHEN a handler fetches external data for `K` through `persistent-entity`'s
  exposed fetch capability
- THEN the handler receives an explicit error, never a silent default or
  empty result

### Requirement: Fetch Attempts Are Observable

Every fetch a handler makes through `persistent-entity`'s exposed
capability MUST be observable through the runtime's existing observability
pipeline (see `external-data-providers`'s observability requirement for
the exact signal set) — `persistent-entity` introduces no separate or
bypassing telemetry path of its own.

#### Scenario: A handler's fetch is observable through the existing pipeline

- GIVEN a handler fetches external data through `persistent-entity`'s
  exposed capability
- WHEN the fetch completes
- THEN a signal is emitted through the runtime's existing observability
  pipeline, never a `persistent-entity`-local or bypassing one

### Requirement: Existing Handlers Unaffected

An existing `PersistentEntity` implementation that never uses the fetch
capability MUST continue to compile and behave exactly as before this
capability exists — the capability is additive and opt-in from the
handler's point of view.

#### Scenario: Unmodified handler compiles and passes unchanged

- GIVEN an existing handler that never uses the fetch capability
- WHEN the workspace is rebuilt after this capability ships
- THEN it compiles and its existing tests pass without modification
