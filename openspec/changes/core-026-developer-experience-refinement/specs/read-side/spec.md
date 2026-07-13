# Spec: CORE-026 Developer Experience Refinement — Read-Side Spawn/Stop Lifecycle

## Capability: read-side (new)

Purpose: `TagSchedulerImpl` gains one call, `spawn_projection`, that wires
the stop/join lifecycle plumbing every consuming application previously
hand-rolled around `run_until_stopped` — spawning the poll loop and returning
a handle whose `stop()` consumes itself, waits for an in-flight batch to
drain, and surfaces a failed drain instead of swallowing it. This capability
specifies only that spawn/stop lifecycle wrapper's observable behavior; the
underlying scheduler engine itself (`TagSchedulerImpl`, `run_until_stopped`,
and friends) is out of scope and unchanged.

**Explicitly not covered by this capability** (see Non-Goals): constructing
a dedup store, an offset store, a tag-discovery mechanism, a handler, or the
application's own queryable read model. All of these remain the calling
application's responsibility, exactly as before this capability exists — a
consuming application still assembles them itself (e.g. reference-app's own
`ReadSideHandles`, unchanged by this capability) and passes them to
`spawn_projection` as arguments.

### Requirement: Ownership Split — Application Owns the Read Model, the Constructor Owns Only the Spawn/Stop Lifecycle

The queryable read model (the application's own domain read-view, e.g. a
`UsersByTenantStore`-shaped type) is constructed and owned by the consuming
application, exactly as before this capability exists — it is application
domain logic, not framework plumbing, and this capability does not wrap,
replace, or return it. `spawn_projection`'s sole responsibility is the
background task's spawn/stop/drain lifecycle: starting the poll loop and
returning a handle that can later stop it and observe how it ended. The
dedup store, offset store, tag-discovery mechanism, handler, and poll
interval are supplied by the caller at the same call site, not constructed
internally.

#### Scenario: The call's result is a lifecycle handle, not a bundled read model

- GIVEN an application has already constructed its own queryable read model,
  dedup store, offset store, and tag-discovery closure
- WHEN it passes them all to `spawn_projection`
- THEN `spawn_projection` returns a poller handle only — the application's
  own read model reference is what it queries directly, not something
  returned or re-wrapped by the call

### Requirement: Spawn/Stop Lifecycle Convenience

`TagSchedulerImpl` MUST expose one call that, given a tag-discovery closure,
poll interval, handler, event store, dedup store, offset store, progress
reporter, and error callback, spawns the poll loop and returns a handle
covering its full stop/drain lifecycle — replacing the stop-signaling and
completion-tracking a caller previously had to hand-roll itself around
`run_until_stopped`. The poll interval MUST be an explicit, required
argument to that same call (not hardcoded, not defaulted, not configured
through a separate setter/builder step) — a caller with different interval
needs (e.g. a fast poll interval in tests) supplies its own value at the
same call site.

#### Scenario: One call yields a spawned poller with full lifecycle plumbing

- GIVEN an application has its dedup store, offset store, tag-discovery
  closure, handler, and event store already constructed
- WHEN it calls `spawn_projection` with those values and an explicit poll
  interval
- THEN the poll loop is spawned and the caller receives a single handle
  covering its stop/drain lifecycle, with no separate stop-signaling or
  completion-tracking left for the caller to hand-roll

#### Scenario: Poll interval is required, not defaulted

- GIVEN two applications with different poll-interval needs (e.g. production
  cadence vs. a fast interval for tests)
- WHEN each calls `spawn_projection`
- THEN each supplies its own interval value at the call site; neither gets a
  silently-hardcoded default it cannot override

### Requirement: Stop Consumes the Handle

The poller handle's stop operation MUST take ownership of the handle (not a
shared or exclusive reference) — once stopped, the handle cannot be reused
or stopped again, making a double-stop a compile-time error rather than a
runtime one.

#### Scenario: A stopped handle cannot be stopped again

- GIVEN a caller holds a poller handle
- WHEN it calls stop on that handle
- THEN the handle is consumed by that call, and no further operation on that
  same handle value is possible

### Requirement: Dynamic Per-Tenant Tag Discovery Preserved

`spawn_projection` MUST call the caller-supplied tag-discovery closure fresh
on each poll, rather than caching its result from the first call — preserving
CORE-018's per-tenant isolation guarantee (one tag stream per tenant) without
regression. This capability does not change what the closure discovers or how
tags are computed — only that `spawn_projection` continues to invoke it per
iteration rather than once at spawn time.

#### Scenario: A tenant's first event is picked up without reconfiguration

- GIVEN a poller handle already spawned, with no prior events for tenant `T`
- WHEN the first event for tenant `T` is written to the event store
- THEN a subsequent poll discovers and processes tenant `T`'s tag without the
  poller being respawned or explicitly told about the new tenant

### Requirement: Graceful Shutdown Preserved

Stopping the spawned poller MUST let any poll batch already in flight finish
draining before the stop call returns, and MUST surface a failed drain to
the caller as an error rather than discarding it silently.

#### Scenario: Stop waits for an in-flight batch to drain

- GIVEN the poller's poll loop is mid-batch when stop is requested
- WHEN the caller calls stop
- THEN stop does not return until that in-flight batch has finished draining

#### Scenario: A failed drain is reported, not swallowed

- GIVEN the spawned poll loop's background task terminates abnormally (panics
  or is aborted) instead of draining cleanly
- WHEN the caller calls stop
- THEN stop returns an error identifying the failure, instead of reporting
  success regardless

### Requirement: Usable By a Real Application Without Escape Hatches

`spawn_projection` MUST be sufficient for a real consuming application to
obtain the same spawned poller it would otherwise hand-wire around
`run_until_stopped`, without needing application-specific escape hatches
beyond supplying its dedup store, offset store, tag-discovery closure,
handler, event store, poll interval, progress reporter, and error callback.

#### Scenario: An application's hand-wired spawn/stop glue migrates to `spawn_projection`

- GIVEN an application previously hand-rolled its own stop-signaling and
  completion-tracking around the scheduler engine's `run_until_stopped` call
  (while still constructing its own dedup store, offset store, tag-discovery
  closure, and read model, as it does today)
- WHEN it calls `spawn_projection` instead, passing those same
  already-constructed values
- THEN it no longer hand-rolls the stop-signaling or completion-tracking
  itself, and per-tenant tag isolation continues to function unchanged; its
  dedup store, offset store, tag-discovery construction, and read model
  ownership are unaffected (see "Ownership Split" above)

### Non-Goals

- No change to `TagSchedulerImpl` or the underlying CORE-005 scheduler/store
  engine's own contract — this capability specifies only the spawn/stop
  wrapper's observable behavior built on top of it. Explicitly unchanged:
  polling semantics (how/when a poll fires), dedup semantics (what counts as
  a duplicate), offset semantics (how progress is tracked and resumed), and
  ordering guarantees (per-tag delivery order) — this capability wraps that
  engine's existing contract, it does not renegotiate any part of it.
- No new persistence format or read-model query capability beyond what
  already exists.
- No change to which type owns the queryable read model — it remains
  entirely application-owned (see "Ownership Split" above); this capability
  does not introduce a framework-owned read-model type.
- **Constructing a dedup store, an offset store, or a tag-discovery
  mechanism is out of scope.** `spawn_projection` takes these as required
  arguments; it does not provide a default or internally construct them.
  An application obtains them exactly as it does today (e.g.
  reference-app's own `ReadSideHandles::new`, itself unchanged by this
  capability) and passes them to `spawn_projection` to spawn the poller.
  A framework-level convenience that also constructs these internally
  (e.g. defaulting to in-memory implementations) was considered and
  rejected — see design.md AD-1, alternative (b) — because the handler and
  tag-discovery closure are irreducibly application-specific, and bundling
  the dedup/offset stores' construction with them would only cover half the
  boilerplate while suggesting the other half was solved too.
- No separate non-spawning "construct" step exists at this capability's
  level — `spawn_projection` always spawns immediately when called. An
  application that needs to construct its read-side wiring without a
  running async runtime (e.g. to assert on its own read model in a
  synchronous test) does so through its own pre-existing constructor (e.g.
  `ReadSideHandles::new`), which this capability does not change or replace.
