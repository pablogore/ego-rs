# Design: CORE-017 — Runtime Infrastructure Integration

## Technical Approach

CORE-017 lands the deferred canonical construction flow in
`RuntimeBuilder` (`crates/service-sdk/src/runtime/`) for exactly one thing:
**config + logger bootstrap**. It does not touch DI resolution, tenant
enforcement, or security wiring — those keep their existing shape. The
Runtime becomes the lifecycle owner of the KITLogger pipeline:

1. A thin `ConfigurationProvider` wraps the already-materialized configuration
   (`serde_json::Value`, produced upstream by `kit_config::ConfigLoader` — the
   same handoff convention `domain`/`persistence`/`transport` already use) and
   exposes the consumed logging view.
2. The **host** — not `RuntimeBuilder` — turns that view into a running
   `Arc<KITLogger>` by calling a public boundary adapter service-sdk exposes.
   This matches CORE-016's own worked example for logging: construct the
   service first (`Logger::new(config.logging)`), then hand the constructed
   service to `RuntimeBuilder`. `RuntimeBuilder::with_logger(Arc<KITLogger>)`
   receives it the same way `.with_security(authn, authz)` receives
   pre-constructed providers — never a config object.
3. `RuntimeBuilder::build()` stays infallible: by the time it runs, the logger
   (if any) is already constructed and initialized. `build()`'s only job for
   this change is registering it on the teardown stack.
4. The constructed `Arc<KITLogger>` is owned by `RuntimeInner` and propagated
   to services through `ServiceContext` (`Option<Arc<KITLogger>>`), mirroring
   exactly the existing `security: Option<Arc<SecurityContext>>` model.
5. On normal `Runtime::shutdown()`, an ordered teardown stack (guarded by a
   `Mutex`, since `RuntimeInner` is always shared via `Arc`) drains every
   initialized component in reverse construction order.

This maps directly to proposal.md (Ownership Model, Lifecycle Flow, Failure
Semantics, ordered teardown). It designs **no** logging or configuration
capability — only integration and lifecycle ownership.

### Grounding finding (drives several decisions below)

The real `kitlogger` API (git checkout `bfb30ae`, branch `develop`) has **no
`KITLogger::from_logging_config`** method. Its public constructors are
`new()`, `default()`, `with_format(kitlogger_formatter::LogFormat)`,
`with_config(telemetry_config_semantics::TelemetryConfig)`, and
`with_exporter_and_format(..)`. Its lifecycle surface is the inherent sync
`init()` / `shutdown()` plus the async `LifecycleAdapter` trait
(`flush()` / `shutdown()`). The `kitlogger` crate root **re-exports none** of
`LogFormat`, `TelemetryConfig`, or `AdapterError`. CORE-017 must not redesign
kitlogger, so the config→logger mapping is a **service-sdk boundary adapter**
built on the constructors that actually exist — not a new kitlogger method.

## Architecture Decisions

### Decision: config→logger mapping lives at the service-sdk boundary, not in kitlogger

| Option | Tradeoff | Decision |
|--------|----------|----------|
| Add `KITLogger::from_logging_config` in kitlogger | Redesigns kitlogger — forbidden by proposal Non Goals | Rejected |
| service-sdk maps a narrow config view → `KITLogger::with_format(..)` | One boundary adapter; kitlogger untouched; honors the one field its simple constructors accept | **Chosen** |
| Use `KITLogger::with_config(TelemetryConfig)` | Would need a `LoggingConfig → TelemetryConfig` translation that does not exist; pulls in `telemetry-config-semantics`; more surface for no CORE-017 benefit | Rejected |

**Rationale**: kitlogger's public constructors accept a **format**; that is the
one piece of `LoggingConfig` CORE-017 can faithfully honor without redesigning
either library. The boundary adapter is `service-sdk`'s own integration glue,
consistent with how each infra crate already adapts a materialized value into
its own type. Richer `LoggingConfig` fields (sampling, buffering, rotation,
redaction) are **not** consumed here — honoring them would mean adding logging
capability, which the proposal forbids. kit-config still owns and validates the
full model upstream. `enabled` is the one exception: it is consumed, but only
to decide *whether to call this adapter at all* (see the `enabled` decision
below) — that is a Runtime-boundary decision, not a logging capability.

This adapter is `pub` in service-sdk and is called by the **host**, before
`RuntimeBuilder::new()` — not by `RuntimeBuilder` itself (see the `with_logger`
decision below).

### Decision: no kit-config dependency in service-sdk — `serde_json::Value` handoff

**Choice**: `ConfigurationProvider` wraps a `serde_json::Value` (materialized
by `kit_config::ConfigLoader` in the host, per CORE-016) and deserializes a
**narrow local view** of the logging subtree; service-sdk does not depend on
kit-config / config-models.
**Alternatives considered**: depend on `config-models` to name `LoggingConfig`
directly (violates the repo-wide convention — `domain/src/config.rs` explicitly
exists "so every infrastructure crate's configuration domain can expose
validation *without any crate depending on kit-config*"); re-derive the full
`LoggingConfig` in service-sdk (duplication, drift).
**Rationale**: Every infra crate (`DatabaseConfig`, `GrpcServerConfig`,
`EntityRuntimeBuilder::from_value`, `EventBusConfig`) receives a materialized
`serde_json::Value` and deserializes its own consumer view. CORE-017 follows
the identical pattern. Structural + module validation stays in kit-config, run
once at load time by the host; the provider stays thin.

### Decision: `ConfigurationProvider` is a concrete struct, not a trait

**Choice**: A single concrete `ConfigurationProvider` wrapping `serde_json::Value`.
**Alternatives considered**: a `trait ConfigurationProvider` with one impl —
rejected (interface with one implementation; add the trait when a second source
actually appears).
**Rationale**: The proposal introduces a thin *role*, not a plugin point. A
concrete struct is enough and keeps the surface minimal. It reconciles the
proposal's "invokes kit-config" wording with CORE-016's frozen "host runs
`ConfigLoader` once" convention: the provider *uses* the kit-config-materialized
value; it owns no sources, merge, or parse logic.

### Decision: `enabled` gates whether the adapter runs at all; no level-based alternative exists

**Choice**: `LoggingSettings.enabled` is consumed by the host-called boundary
adapter, not by `RuntimeBuilder`: `if !settings.enabled { skip build_logger,
pass nothing to .with_logger(..) }`. When `enabled` is absent/true, the
existing format-mapping path runs unchanged.

**Alternatives considered**: express "off" purely through a log level/severity
(e.g. `Severity::Off`) so there is a single mechanism instead of two. Verified
against the real `kitlogger` source (git checkout `bfb30ae`) and **rejected**:
`Severity` (`kitlogger-log-domain`) has no `Off` variant (`Trace..Fatal` only),
and neither `log()` nor `log_record()` on `KITLogger` check any threshold
before exporting — not even `with_config(TelemetryConfig)`'s computed
`effective_state` (which does model `Disabled`) is consulted by them. A
level-based "off" is not implementable against the current kitlogger API.

**Rationale**: kitlogger has no internal suppression mechanism today, so
`enabled` is not competing with an alternative one — it is the *only* real
on/off switch available, expressed as "construct the logger or don't," the
same binary `security: Option<Arc<SecurityContext>>` already models. This is a
Runtime-boundary decision, not new logging capability.

### Decision: `RuntimeBuilder` receives a pre-built logger via `with_logger`; `build()` stays infallible

**Choice**: `RuntimeBuilder::with_logger(mut self, logger: Arc<KITLogger>) -> Self`,
mirroring `.with_security(authn, authz)` exactly. `build(self) -> Runtime` is
**not** `Result`-returning. The logger is optional — a runtime with no
`.with_logger(..)` call is still valid, the same way one with no security
providers is.

**Alternatives considered**: `RuntimeBuilder::with_configuration_provider(provider)`
with construction happening inside `build()` — **rejected**. CORE-016's frozen
spec uses logging as its own worked example of what `RuntimeBuilder` must NOT
accept: `with_logging_config(...)` is explicitly the forbidden shape; the
canonical shape is `let logger = Logger::new(config.logging);
RuntimeBuilder::new().with_logger(logger)`. Passing a `ConfigurationProvider`
(which still carries an undeserialized `serde_json::Value`) into `RuntimeBuilder`
is the same violation with an extra layer of naming — `RuntimeBuilder` would
still be the one deserializing and constructing from raw configuration.

**Where construction actually happens**: the **host** calls
`ConfigurationProvider::from_value(..)` → `.logging()` → the public
`build_logger(&LoggingSettings)` adapter, **before** ever calling
`RuntimeBuilder::new()`. Both can fail (`RuntimeInfraError::ConfigInvalid`,
`RuntimeInfraError::LoggerInit`) — that failure now stops host bootstrap
outright, before `RuntimeBuilder` exists, which is a *stronger* fail-fast
guarantee than failing partway through `build()`. This also matches CORE-016's
"configuration materialization completes before runtime construction begins."

**Consequence for `build()`**: by the time `RuntimeBuilder::build()` runs, the
logger (if supplied) is already constructed and initialized — `build()`'s only
job is pushing it onto the teardown stack, which cannot fail. No existing
`RuntimeBuilder::new().build()` call site needs to change; this is a smaller
migration than the earlier fallible-`build()` shape. If a genuinely fallible
step is ever added *inside* `build()` itself, `build()` becomes `Result`-returning
at that point — not speculatively now.

### Decision: reverse-order teardown stack; flush realized via the console exporter's flush-on-shutdown

| Option | Tradeoff | Decision |
|--------|----------|----------|
| Sync inherent `KITLogger::shutdown()` in LIFO order | Zero new async surface; console exporter uses `OnShutdownFlush`, so shutdown flushes then closes — satisfies "flush → close, no lost logs" | **Chosen** |
| Explicit async `LifecycleAdapter::flush().await` then `shutdown().await` | Names the flush step separately, but forces `build()`/`shutdown()` async + a `telemetry-adapter-contracts` dependency for a guarantee `OnShutdownFlush` already gives | Rejected (see Risks) |

**Rationale**: kitlogger's console exporter is constructed with the
`OnShutdownFlush` strategy, so its inherent sync `shutdown()` performs
**flush-then-close** internally — exactly the proposal's "flush pipeline →
close outputs" ordering, without a separate async flush call or an extra
dependency. The Runtime's contribution is guaranteeing the reverse
**construction order** across components, which a LIFO stack provides. Today
the stack holds one entry (the logger).

**Mutability**: `RuntimeInner` is always accessed through `Arc<RuntimeInner>`
(generated proxies hold `Weak<RuntimeInner>`), so `Runtime::shutdown(&self)`
cannot call a `&mut self` drain method directly. `RuntimeInner` stores
`Mutex<TeardownStack>`; `shutdown()` does
`self.teardown.lock().expect("teardown mutex poisoned").drain()`. A poisoned
lock (only possible if a prior `shutdown()` panicked mid-drain) is treated as
a hard error rather than silently recovered — consistent with "no degraded
mode."

## Data Flow

```
Host (downstream app / example / test) — all of this runs BEFORE RuntimeBuilder::new()
  kit_config::ConfigLoader (external, invoked ONCE) → materialized serde_json::Value
        │
        ▼
  ConfigurationProvider::from_value(root)        (thin — no sources/merge/parse)
        │  .logging()  → deserialize narrow view ── serde error ─► ConfigInvalid ─► host bootstrap FAILS
        ▼
  LoggingSettings { enabled, format }
        │  enabled == false ──────────────────────────────────────► skip: no logger built
        ▼  enabled == true
  build_logger(&settings)   (pub adapter, service-sdk)
        │  map LogFormatSetting → kitlogger_formatter::LogFormat
        │  KITLogger::with_format(fmt)
        │  logger.init()  ── AdapterError ─► LoggerInit ─► host bootstrap FAILS
        ▼
   Arc<KITLogger>  (or nothing, if disabled/absent)
        │
        ▼
  RuntimeBuilder::new()
      .with_logger(logger)     (optional — receives a fully-constructed service, CORE-016 rule intact)
      .with_security(authn, authz)               (unchanged)
      .build()                                    (infallible — pushes logger onto TeardownStack)
        ▼
     Runtime (running)   RuntimeInner owns Arc<KITLogger> + Mutex<TeardownStack>
        │  entry point builds ServiceContext::new().with_logger(rt.logger())
        │  services call ctx.logger()  →  Option<&KITLogger>
        ▼
  Runtime::shutdown(): lock Mutex<TeardownStack> → drain in reverse → shutdown() (flushes via OnShutdownFlush)
```

## File Changes

| File | Action | Description |
|------|--------|-------------|
| `crates/service-sdk/Cargo.toml` | Modify | Add `serde`, `serde_json` (real deps) and `kitlogger-formatter` (git, same repo/branch as `kitlogger`) for `LogFormat` |
| `crates/service-sdk/src/runtime/config_provider.rs` | Create | `ConfigurationProvider` (wraps `serde_json::Value`) + narrow `LoggingSettings`/`LogFormatSetting` view + `logging()`; `pub` — called by the host |
| `crates/service-sdk/src/runtime/logger.rs` | Create | `pub fn build_logger(&LoggingSettings) -> Result<Option<Arc<KITLogger>>, RuntimeInfraError>` — the canonical logger construction entry point (format mapping + `init()`; `Ok(None)` when `enabled == false`), called by the host before `RuntimeBuilder::new()`; `TeardownStack` |
| `crates/service-sdk/src/runtime/builder.rs` | Modify | `with_logger(Arc<KITLogger>)` (mirrors `with_security`); `build()` stays `-> Runtime` (infallible); `Runtime::shutdown()` |
| `crates/service-sdk/src/runtime/runtime_builder.rs` | Modify | `RuntimeInner` gains `logger: Option<Arc<KITLogger>>` + `Mutex<TeardownStack>`; `logger()` accessor (`#[doc(hidden)]`, macro-facing, per AD-7); resolves the "deferred to TASK-013/014" construction note for config+logger |
| `crates/service-sdk/src/runtime/mod.rs` | Modify | Re-export `ConfigurationProvider`, `LoggingSettings`, `LogFormatSetting`, `RuntimeInfraError`, `build_logger` — this is the full host-facing public surface |
| `crates/service-sdk/src/context/mod.rs` | Modify | Add `logger: Option<Arc<KITLogger>>`, `with_logger(..)`, `logger()` — access only |
| `crates/service-sdk/src/error/` (or `runtime_builder.rs`) | Create/Modify | `RuntimeInfraError` (thiserror) — construction/lifecycle failures |

## Interfaces / Contracts

```rust
// --- ConfigurationProvider: thin host-boundary role ---------------------------
// Holds the config already materialized by kit_config::ConfigLoader (host side).
// It owns no sources/merge/parse — it only exposes the consumed logging view.
// Called by the HOST, before RuntimeBuilder::new() — never by RuntimeBuilder itself.
pub struct ConfigurationProvider {
    root: serde_json::Value,
}

impl ConfigurationProvider {
    pub fn from_value(root: serde_json::Value) -> Self { Self { root } }

    /// Deserialize the consumed logging view. Any structural error is fail-fast.
    pub fn logging(&self) -> Result<LoggingSettings, RuntimeInfraError> {
        let node = self.root.get("logging").cloned().unwrap_or_default();
        serde_json::from_value(node)
            .map_err(|e| RuntimeInfraError::ConfigInvalid { reason: e.to_string() })
    }
}

/// Narrow consumer view — NOT a reimplementation of kit-config's LoggingConfig.
/// Only the fields CORE-017 actually consumes. kit-config owns the full model.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(default)]
pub struct LoggingSettings {
    pub enabled: bool,           // gates whether build_logger runs at all — see enabled decision
    pub format: LogFormatSetting,
}

#[derive(Debug, Clone, Copy, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LogFormatSetting { Json, Pretty, Compact, Text } // mirrors config-models strings

// --- Boundary adapter: config view → real KITLogger constructor --------------
// This is the canonical PUBLIC logger construction entry point — the only
// supported public way to turn a LoggingSettings view into a KITLogger. The
// restriction is on the public API surface; a private internal helper used
// only by this function's own implementation would not violate it. Do not add
// another *public* helper that constructs KITLogger by a different path.
// `pub`: called by the host, before RuntimeBuilder::new(). Returns `Ok(None)`
// when `enabled == false` — the only "off" mechanism kitlogger's API supports.
pub fn build_logger(cfg: &LoggingSettings) -> Result<Option<Arc<KITLogger>>, RuntimeInfraError> {
    if !cfg.enabled {
        return Ok(None);
    }
    use kitlogger_formatter::LogFormat;
    let fmt = match cfg.format {
        LogFormatSetting::Json    => LogFormat::Json,
        LogFormatSetting::Pretty  => LogFormat::HumanReadable,
        LogFormatSetting::Compact => LogFormat::Text,
        LogFormatSetting::Text    => LogFormat::Text,
    };
    let logger = KITLogger::with_format(fmt);
    // KITLogger::init() -> Result<(), AdapterError>; AdapterError is not re-exported,
    // so map by Debug at the boundary rather than naming the type.
    logger.init().map_err(|e| RuntimeInfraError::LoggerInit { reason: format!("{e:?}") })?;
    Ok(Some(Arc::new(logger)))
}

// --- Host bootstrap (illustrative — not RuntimeBuilder's responsibility) -----
// let provider = ConfigurationProvider::from_value(materialized_json);
// let settings = provider.logging()?;                 // ConfigInvalid → host bootstrap fails
// let logger = build_logger(&settings)?;               // LoggerInit → host bootstrap fails
// let mut builder = RuntimeBuilder::new().with_security(authn, authz);
// if let Some(logger) = logger { builder = builder.with_logger(logger); }
// let runtime = builder.build();                        // infallible

// --- RuntimeBuilder: receives fully-constructed services only (CORE-016) -----
impl RuntimeBuilder {
    pub fn with_logger(mut self, logger: Arc<KITLogger>) -> Self {
        self.logger = Some(logger);
        self
    }

    pub fn build(self) -> Runtime {
        let mut teardown = TeardownStack::new();
        if let Some(logger) = &self.logger {
            teardown.push(logger.clone()); // register for reverse-order teardown
        }
        Runtime { inner: Arc::new(RuntimeInner::new_with_logger(
            self.registry, self.interceptor_chain, self.security_providers(),
            self.logger, Mutex::new(teardown),
        ))}
    }
}

impl Runtime {
    /// Drains initialized infrastructure in reverse construction order.
    /// For the console exporter, shutdown() flushes (OnShutdownFlush) then closes.
    pub fn shutdown(&self) -> Result<(), RuntimeInfraError> {
        self.inner.teardown.lock().expect("teardown mutex poisoned").drain()
    }
}

// --- Ordered teardown: LIFO over the console-exporter's flush-on-shutdown -----
// Held behind `Mutex<TeardownStack>` in RuntimeInner — RuntimeInner is always
// shared via Arc (see Weak<RuntimeInner> in generated proxies), so shutdown(&self)
// needs interior mutability to drain it.
struct TeardownStack { entries: Vec<Arc<KITLogger>> }
impl TeardownStack {
    fn new() -> Self { Self { entries: Vec::new() } }
    fn push(&mut self, l: Arc<KITLogger>) { self.entries.push(l); }
    /// Reverse construction order. Collects the first error but shuts down all.
    /// Idempotent: a second call drains an already-empty stack and returns `Ok(())`.
    fn drain(&mut self) -> Result<(), RuntimeInfraError> {
        let mut first_err = None;
        while let Some(l) = self.entries.pop() {                 // LIFO = reverse order
            if let Err(e) = l.shutdown() {                       // flush-then-close (OnShutdownFlush)
                first_err.get_or_insert(RuntimeInfraError::Teardown { reason: format!("{e:?}") });
            }
        }
        first_err.map_or(Ok(()), Err)
    }
}

// --- ServiceContext: access only (mirrors `security: Option<Arc<..>>`) -------
impl ServiceContext {
    pub fn with_logger(mut self, logger: Arc<KITLogger>) -> Self { self.logger = Some(logger); self }
    pub fn logger(&self) -> Option<&KITLogger> { self.logger.as_deref() }
}

// --- Failure semantics as concrete variants (thiserror, per domain convention)
// Only variants the real APIs can actually produce: ConfigInvalid (serde, in
// ConfigurationProvider::logging()) and LoggerInit (AdapterError, in build_logger's
// logger.init() call) cover host bootstrap; Teardown covers Runtime::shutdown().
// kitlogger's init() does not expose a separately-distinguishable "output" failure
// stage, so there is no OutputInit variant — it would be unreachable.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RuntimeInfraError {
    #[error("invalid configuration: {reason}")]
    ConfigInvalid { reason: String },
    #[error("logger initialization failed: {reason}")]
    LoggerInit { reason: String },
    #[error("infrastructure teardown failed: {reason}")]
    Teardown { reason: String },
}
```

## Testing Strategy

| Layer | What to Test | Approach |
|-------|-------------|----------|
| Unit | `ConfigurationProvider::logging()` parses a valid view; malformed `format`/type → `ConfigInvalid` | in-memory `serde_json::json!` values |
| Unit | `build_logger` maps each `LogFormatSetting` to the expected `kitlogger_formatter::LogFormat` | table test on the match arms |
| Unit | `build_logger` with `enabled: false` → `Ok(None)`, no `KITLogger` constructed | assert `None`, no side effect |
| Unit | `build_logger` with `init()` failure → `Err(LoggerInit)` | inject via a failing exporter if kitlogger's test seams allow it, else document as untestable at this layer |
| Unit | `RuntimeBuilder::build()` with no `.with_logger(..)` call → `logger()` is `None`; with one → `logger()` is `Some` | construct builder, assert |
| Unit | `TeardownStack::drain` shuts down in reverse (LIFO) order | stub/verify order with a capture logger via `with_exporter_and_format` |
| Unit | `TeardownStack::drain` is idempotent — a second call on an already-drained stack returns `Ok(())` | call `drain()` twice, assert both succeed |
| Integration | build → obtain logger via `ServiceContext::with_logger` → `Runtime::shutdown()` flushes; no lost records | capture-buffer exporter (`ConsoleExporterImpl::set_writers`) |

## Migration / Rollout

Purely additive — no existing call site changes. `RuntimeBuilder::build()`
keeps its current infallible signature; `.with_logger(..)` is a new optional
builder method, same shape as `.with_security(..)`. `ServiceContext` gains one
optional field — `new()` defaults it to `None`, so all existing constructors
and clones keep compiling. `kitlogger` stays a dependency; `kitlogger-formatter`
is added from the same git repo/branch. kit-config and kitlogger are
unmodified and keep compiling standalone. Logger construction and the
`enabled`/config-invalid failure paths are new host-side responsibility, not
something existing `RuntimeBuilder` callers are forced to adopt.

## Open Questions — for the Tasks phase

- [ ] **New git dependencies resolve.** `kitlogger-formatter` is a sibling
  member of the kitlogger workspace; confirm it resolves as a git dependency
  (same URL/branch as `kitlogger`) during `cargo build`.
- [x] **`AdapterError` mapping — resolved.** Confirmed against the kitlogger
  source (`telemetry-adapter-contracts::error::AdapterError`): it derives
  `Debug`, so the boundary's `{:?}` mapping in `build_logger` compiles as
  specified. No further verification needed. If a richer message is wanted
  later, add `telemetry-adapter-contracts` and match on variants — not needed
  for v1.

## Future Considerations

Not open questions — no decision is pending on either of these; they're
deferred work, noted so they aren't mistaken for guarantees this change makes.

- **Explicit async flush.** If a future exporter does *not* use
  `OnShutdownFlush`, "flush before close" would need an explicit
  `LifecycleAdapter::flush().await` before `shutdown()`, making `build()` /
  `shutdown()` async and adding `telemetry-adapter-contracts`. Out of scope for
  CORE-017's console exporter.
- **`RuntimeInner::new()` / `RuntimeInner::default()` stay `pub`.**
  Pre-existing bypass of `RuntimeBuilder` (noted in `runtime_builder.rs`'s own
  TASK-014 comment) — a caller can still construct a bare `RuntimeInner` with
  no logger and an empty teardown stack directly. CORE-017 does not close this.
