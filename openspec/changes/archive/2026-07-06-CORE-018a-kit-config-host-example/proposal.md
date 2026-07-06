# Proposal: CORE-018a — Real kit-config Host Example in reference-app

Tracks GitHub issue #119.

## Intent

The CORE-016 frozen constraint — "RuntimeBuilder MUST NOT receive raw configuration
values" (`openspec/changes/archive/2026-07-03-CORE-016-app-config-model/spec.md:148`)
— has only ever been demonstrated with hand-simulated JSON
(`crates/service-sdk/examples/logging_bootstrap.rs` builds `json!({...})` directly).
No code in ego-rs proves the contract end-to-end with a real config source.
`examples/reference-app` is the composition root the CORE-016 audit
(`docs/core-016-config-audit.md`) says kit-config should land at, yet its
`build_runtime()` wires only security providers and its doc comment still claims
kit-config is "intentionally out of scope".

## What Changes

- Add `kit-config` to `examples/reference-app/Cargo.toml` as a git dependency,
  using the same git-dep strategy already used for kitlogger in service-sdk
  (track the branch current at apply time — no path dependency).
- Extend `build_runtime()` in `examples/reference-app` so real kit-config output
  flows into `ConfigurationProvider`: `kit-config` → materialized configuration
  → `ConfigurationProvider` → `build_logger` → `.with_logger(...)`. Exact
  conversion mechanics (types, intermediate steps) are a Design concern.
- Rewrite the stale "kit-config is intentionally out of scope" doc comment in
  `examples/reference-app/src/lib.rs`.
- The example documents the current precedence behavior provided by kit-config
  (file-based sources currently outrank environment/key-value sources, and env
  vars cannot populate or override the nested `logging` object). This is
  observed behavior of kit-config today, not a contract ego-rs guarantees —
  explicit comment/doc note only, no workaround code.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `service-sdk`: add a requirement that the reference host example materializes
  configuration through real `kit_config::ConfigLoader` at the composition root
  and hands only materialized configuration (via `ConfigurationProvider`) to
  `RuntimeBuilder` — never raw config sources.

## Non-Goals

- No changes to `crates/service-sdk` (ConfigurationProvider is already correct per
  CORE-016/017) or to kit-config itself.
- `crates/service-sdk/examples/logging_bootstrap.rs` stays exactly as-is — it is a
  framework-level unit illustration, not a host demo.
- No new example crate or bin — extend `reference-app` only.
- Logging config only: no DB config stub, no other typed views, no custom
  `ConfigurationSource` for nested env overrides.

## Affected Areas

| Area | Impact | Description |
|------|--------|-------------|
| `examples/reference-app/Cargo.toml` | Modified | Add kit-config git dependency |
| `examples/reference-app/src/lib.rs` | Modified | Real ConfigLoader wiring in `build_runtime()`; doc comment rewrite; precedence-limitation note |
| `examples/reference-app/` (config file) | New | TOML file the example loads |

## Risks

| Risk | Likelihood | Mitigation |
|------|------------|------------|
| CI lacks access to private `pablogore/kit-config` repo | Low | Same access pattern already proven for kitlogger git deps; confirm, don't assume |
| kit-config builder API differs from exploration notes | Low | Verify compatibility with the current kit-config public API during tasks/apply |

## Rollback Plan

Revert the commit. Example-only change: one dependency line, one crate's wiring,
one config file. No framework API, data, or consumer migration to unwind.

## Dependencies

- External: `github.com/pablogore/kit-config` (branch current at apply time) reachable at build time.

## Success Criteria

- [ ] `examples/reference-app` builds and its tests pass with real kit-config loading.
- [ ] `build_runtime()` demonstrates kit-config → materialized configuration →
      `ConfigurationProvider` → `build_logger()` → `RuntimeBuilder`.
- [ ] No unresolved configuration source reaches `RuntimeBuilder`.
- [ ] Stale "out of scope" doc comment is gone; precedence limitation is documented.
- [ ] `crates/service-sdk` and `logging_bootstrap.rs` show zero diff.
