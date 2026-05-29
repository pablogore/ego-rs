## [ ] 1. Observability Port Trait

- [ ] 1.1 Define `Observability` trait in `crates/domain/` — `fn trace(&self, event: SemanticEvent)`, `fn metric(&self, name: &str, value: f64)`, `fn log(&self, level: Level, message: &str)`
- [ ] 1.2 Define `SemanticEvent` struct — event_name, correlation_id, actor_id, lifecycle_state, timestamp, metadata map
- [ ] 1.3 Define `Level` enum — Debug, Info, Warn, Error
- [ ] 1.4 Observability SHALL be non-mutating — events/calls MUST NOT alter runtime state

## [ ] 2. In-Memory Adapter

- [ ] 2.1 Implement `InMemoryObservability` in `crates/infrastructure/` — implements `Observability` trait
- [ ] 2.2 Collect traces, metrics, logs in memory for test inspection
- [ ] 2.3 Export `get_traces()`, `get_metrics()`, `get_logs()` for test verification

## [ ] 3. Noop Adapter

- [ ] 3.1 Implement `NoopObservability` in `crates/infrastructure/` — implements `Observability` trait, discards all events
- [ ] 3.2 Minimal overhead — no allocation, no storage

## [ ] 4. Tests

- [ ] 4.1 Test: trace event recorded → inspectable via `get_traces()`
- [ ] 4.2 Test: metric recorded → inspectable via `get_metrics()`
- [ ] 4.3 Test: log recorded → inspectable via `get_logs()`
- [ ] 4.4 Test: observability is non-mutating — recording trace does not change actor state
- [ ] 4.5 Test: NoopObservability discards all events without allocation
- [ ] 4.6 Test: deterministic — identical semantic events produce identical observable traces (mock only)

## [ ] 5. Verification

- [ ] 5.1 Run `cargo test --workspace` — all tests pass
- [ ] 5.2 Run `cargo clippy --workspace -- -D warnings` — no warnings
- [ ] 5.3 Verify trait lives in `crates/domain/` (hexagonal: domain defines port)
- [ ] 5.4 Verify adapters live in `crates/infrastructure/` (hexagonal: infrastructure implements port)
- [ ] 5.5 Verify no vendor-specific dependencies (no OpenTelemetry, Prometheus, Datadog, etc. in domain)