# Proposal: CORE-017 — Runtime Infrastructure Integration

## Context

Two infrastructure libraries are already complete and work independently:

- **kit-config** — configuration model, loading, structural validation. Referenced today
  via the established `serde_json::Value` handoff pattern (`crates/domain/src/config.rs`,
  `crates/persistence/src/config.rs`, `crates/transport/src/config.rs`,
  `crates/persistent-entity/src/builder.rs`, `crates/ego-scheduler/src/event_bus.rs`).
  Not a workspace crate; treated as an external dependency.
- **kitlogger** — the logging pipeline. Already a real dependency of `service-sdk`
  (`crates/service-sdk/Cargo.toml`, git dependency on branch `develop`).

What does NOT exist yet: anyone who owns their **lifecycle**. Nothing bootstraps them
together, gates runtime construction on valid configuration, constructs the logging
pipeline, or flushes and shuts it down in order. `RuntimeBuilder`
(`crates/service-sdk/src/runtime/builder.rs`) exists as scaffolding, and
`crates/service-sdk/src/runtime/runtime_builder.rs` explicitly defers its canonical
construction flow.

CORE-017 is a **runtime integration** capability. It is not a logging feature and not a
configuration feature. It makes the Ego Runtime the owner of the
bootstrap → validate → construct → run → flush → shutdown lifecycle for these two
libraries — nothing else.

---

# Primary Goal

Define the infrastructure lifecycle contract. The Runtime architecture owns this
contract; the **host** executes the steps that produce a fully-constructed
logger, and the **Runtime** owns everything from receiving it onward:

1. **Bootstrap** — the host loads configuration through a thin `ConfigurationProvider`
   and constructs the logger, before the Runtime is built.
2. **Validation gate** — invalid configuration stops host bootstrap; the Runtime
   never starts existing.
3. **Construction** — the host builds the KITLogger pipeline from validated
   configuration and hands it to `RuntimeBuilder` already constructed.
4. **Access** — services reach the logger through `ServiceContext`, never by constructing it.
5. **Flush / shutdown** — the Runtime drains and closes the pipeline in a defined order.

This proposal finally lands the canonical config + logger bootstrap flow —
host-executed construction, `RuntimeBuilder`-owned lifecycle from there on —
resolving the "deferred to TASK-013/TASK-014" construction notes in
`runtime_builder.rs`.

---

# Architectural Principle

The Runtime owns infrastructure lifecycle **orchestration**; kit-config and KITLogger
remain the owners of their respective domains. The Runtime does not absorb their
capabilities — it only coordinates when they start, whether they are valid, and when
they stop. Services consume; they never construct.

---

# Ownership Model

| Component | Owns | Status |
|---|---|---|
| `ConfigurationProvider` | Configuration loading at the host bootstrap boundary, before Runtime construction begins | New — thin role introduced by THIS change (no such type exists in the tree today) |
| kit-config | Configuration model + structural validation | External, untouched |
| KITLogger | Logging pipeline | External, untouched |
| `Runtime` | Lifecycle — init / flush / shutdown ordering | New responsibility, THIS change |
| `ServiceContext` | Access for services | Exists (`crates/service-sdk/src/context/mod.rs`), extended for access only |

Hard rule: **services never construct loggers directly.** No global logger, no
singleton, no service locator. Logging access follows the same explicit-propagation
ownership model `ServiceContext` already documents: dependencies are passed forward,
never looked up ambiently.

### On the `ConfigurationProvider` name

CORE-016's Non Goals said "Do NOT design ConfigurationProvider" — that referred to the
loader/source abstraction pair reimplemented in the deleted `ego-config-sdk`
(see `docs/core-016-config-audit.md`). CORE-017's `ConfigurationProvider` is not that:
it is a thin boundary role that invokes kit-config and hands validated configuration to
whoever constructs the logger. It owns no parsing, no sources, no merge logic — those
remain in kit-config, per CORE-016.

### Relation to CORE-016's Runtime rule

CORE-016 froze the rule "RuntimeBuilder MUST NOT accept raw configuration objects; it
only ever receives fully-constructed services" — with logging as its own worked
example: NOT allowed is `RuntimeBuilder::new().with_logging_config(...)`; canonical is
`let logger = Logger::new(config.logging); RuntimeBuilder::new().with_logger(logger)`.

CORE-017 follows that example directly rather than carving out an exception for its own
infrastructure bootstrap. The `KITLogger` pipeline is constructed and initialized by the
**host**, through a `ConfigurationProvider`-backed boundary adapter that service-sdk
exposes — *before* `RuntimeBuilder::new()` is ever called. `RuntimeBuilder` only ever
receives the already-constructed `Arc<KITLogger>` via `.with_logger(...)`, the same
shape as `.with_security(authn, authz)`. Configuration materialization completes before
runtime construction begins, exactly as CORE-016 requires — no exception needed.

---

# Lifecycle Flow

```
Host bootstrap (not RuntimeBuilder)
        │  ConfigurationProvider::from_value(config already materialized by kit-config)
        ▼
  provider.logging() ── invalid ──► host bootstrap FAILS, RuntimeBuilder never runs
        │
        ▼
  build_logger(settings) ── failure ──► host bootstrap FAILS, RuntimeBuilder never runs
        │  (constructs + initializes the KITLogger pipeline)
        ▼
   Arc<KITLogger>
        │
        ▼
 RuntimeBuilder::new().with_logger(logger)   ← receives a fully-constructed service (CORE-016)
        │
        ▼
 RuntimeBuilder::build()
        │  registers logger for ordered teardown
        ▼
     Runtime (running)
        │  services log via ServiceContext
        ▼
  shutdown: drain teardown stack in reverse → flush pipeline (OnShutdownFlush) → close → stop
```

---

# Failure Semantics

Fail-fast. There is never a partially initialized runtime.

| Failure | Result |
|---|---|
| Invalid configuration | Host bootstrap fails; `RuntimeBuilder` is never invoked |
| Logger initialization failure | Host bootstrap fails; `RuntimeBuilder` is never invoked |
| A later infrastructure step fails during `RuntimeBuilder::build()` | Already-initialized infrastructure is torn down in reverse order; construction fails |

No degraded mode. No silent fallback. A runtime either constructs fully or returns an
error before any service runs. Because configuration materialization and logger
construction happen before `RuntimeBuilder::new()` (per CORE-016), the first two
failure modes never even reach the Runtime — they are stopped at the host boundary,
which is a stronger guarantee than failing inside construction.

**Ordered teardown on late failure.** Once the logging pipeline is constructed, any
later failure during Runtime construction (e.g. output initialization, or a later
bootstrap step this proposal does not itself define) MUST flush and shut down every
already-initialized piece of infrastructure, in reverse construction order, before the
error is returned. Aborting without teardown is not acceptable — it leaks resources and
leaves the construction contract undefined for callers.

---

# Non Goals

Do NOT design or introduce:

- tracing
- metrics
- OpenTelemetry
- authorization / authentication
- runtime telemetry
- distributed tracing
- service discovery
- configuration providers beyond current kit-config capabilities
- hot reload
- dynamic reconfiguration

Do NOT redesign kit-config or kitlogger. No new logging or configuration capabilities —
integration and lifecycle ownership only.

---

# Constraints

- kit-config and kitlogger remain independent, reusable libraries after this change.
- No global logger, no singleton, no service locator — explicit dependency ownership only.
- Runtime remains the canonical lifecycle owner.
- `ConfigurationProvider` stays thin: it invokes kit-config; it never reimplements it.
- The CORE-016 business-configuration model is unchanged.

---

# Verification Notes

Checked before freezing this proposal:

- **ADR conflicts** — the only ADR reference in the repo is ADR-009 in
  `openspec/specs/domain/auth.md` (authentication scope; unrelated to this change).
  No ADR-008 or ADR-010 exists in this repo; none is invented here.
- **Duplicate responsibilities** — `ConfigurationProvider` / `ConfigLoader` have zero
  live definitions in the tree (confirmed by grep and by
  `docs/core-016-config-audit.md`); the thin role introduced here duplicates nothing.
- **New canonical models** — none beyond the thin `ConfigurationProvider` role described
  above.

---

# Success Criteria

- The host has one canonical bootstrap flow that materializes configuration and
  constructs the logging pipeline, failing fast (before `RuntimeBuilder` runs)
  on any invalid input; `RuntimeBuilder` has one canonical way to receive the
  result (`.with_logger(..)`).
- A service obtains logging through `ServiceContext`; grep finds no direct KITLogger
  construction in service code.
- Runtime shutdown flushes the logging pipeline before process exit — no lost log records
  on clean shutdown.
- kit-config and kitlogger compile and function standalone, unmodified.

---

# Scope Boundaries

Confirmed layering for this change:

- **Proposal** — integration architecture, ownership, lifecycle, failure semantics (this document).
- **Spec** — observable behavior, exact contracts, exact signatures.
- **Design** — implementation strategy, concrete APIs, module layout.

---

# Deliverables

Produce only:

- proposal.md

Do NOT produce:

- spec.md
- design.md
- tasks.md

---

# ✅ Proposal Frozen

Ownership, lifecycle ordering, and failure semantics are consistent with CORE-016 and
the existing runtime scaffolding. Ready for:

- Spec
- Design
- Tasks
