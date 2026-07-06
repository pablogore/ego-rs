# Design: CORE-018a — Real kit-config Host Example in reference-app

## Technical Approach

This is an **example-crate wiring change**, no framework code touched. It
makes `examples/reference-app` exercise for real the host-side path CORE-017
designed only in the abstract (`serde_json::Value` handoff) and CORE-016's
frozen "RuntimeBuilder MUST NOT receive raw configuration" rule proved only
with a hand-built `json!` in `logging_bootstrap.rs`.

`build_runtime()` gains a config-materialization prologue that runs the real
`kit_config::ConfigLoader`, converts its output to `serde_json::Value`, feeds
it through the already-public service-sdk boundary (`ConfigurationProvider` →
`build_logger` → `.with_logger(..)`), and hands `RuntimeBuilder` only the
constructed `Arc<KITLogger>` — never a config object. AppConfig's existing
CORE-016 typed-construction demo stays exactly as-is; the kit-config path is
additive and scoped to the logging subtree only (Non-Goals).

## Implementation Sketch

Verified concrete flow (against the real kit-config source, not exploration
notes):

```
kit_config::ConfigLoader::builder()          // config-loaders/src/loader.rs:89
    .add_defaults()                           // priority 0  (yields {})
    .add_environment()                        // priority 50 (flat String keys)
    .add_toml(<abs path>)                     // priority 200, non-optional
    .build()?                                 // -> Result<ConfigLoader, kit_config::ConfigError>
    .load()?                                  // -> Result<HashMap<String, serde_json::Value>, _>
        │
serde_json::to_value(map)?                    // HashMap -> Value::Object
        │
ConfigurationProvider::from_value(value)      // service-sdk, already public
    .logging()?                               // -> LoggingSettings | ConfigInvalid
        │
build_logger(&settings)?                      // -> Option<Arc<KITLogger>> | LoggerInit
        │
RuntimeBuilder::new().with_security(authn, authz)
    [.with_logger(logger) if Some]
    .build()                                  // infallible (CORE-017)
```

## Architecture Decisions

### Decision: git dependency, default features, no branch pin in the design

**Choice**: `kit-config = { git = "https://github.com/pablogore/kit-config.git", branch = "<current at apply>" }` in `examples/reference-app/Cargo.toml`, mirroring service-sdk's kitlogger git deps.
**Alternatives**: path dep (rejected — kit-config is a separate repo, not a workspace member); pin a branch name here (rejected — ages the design when kit-config's default branch changes, same reasoning as the proposal).
**Rationale**: Same access pattern already proven for kitlogger. Default features (`config-loaders`, `logging`) give `kit_config::ConfigLoader` + `kit_config::ConfigError`; no feature flags needed. Also add `serde_json = "1"` (needed for `to_value`; service-sdk does not re-export it).

### Decision: TOML anchored to `CARGO_MANIFEST_DIR`, not CWD-relative

**Choice**: new file `examples/reference-app/config.toml`, loaded via `.add_toml(concat!(env!("CARGO_MANIFEST_DIR"), "/config.toml"))`.
**Alternatives**: bare `"config.toml"` (rejected — `TomlFileSource` reads through `fs::read_to_string(&path)` relative to process CWD; robust for `cargo run` but fragile the moment a test or tool runs from a different CWD).
**Rationale**: `env!` resolves at compile time to an absolute path, so the example and its test load the same file regardless of CWD. One macro, no abstraction. Minimal `[logging]` table — the only subtree consumed:

```toml
[logging]
enabled = true
format  = "pretty"
```

`DefaultsSource` yields `{}`, so `[logging]` must live in the file: an absent
`logging` subtree deserializes to `ConfigInvalid` (per
`config_provider.rs` test `logging_missing_subtree_is_config_invalid`).

### Decision: keep `add_environment()`, document the flat-source limitation in code

**Choice**: include `.add_environment()` in the chain even though it can never
populate/override nested `logging`.
**Rationale**: This IS the behavior being demonstrated. `EnvironmentSource`
(priority 50) emits flat lowercased `Value::String` top-level keys; the TOML
source (priority 200) applies later and wins; neither env nor key-value sources
can build a nested `logging.*` object. A `// ponytail:` comment states this as
**observed kit-config behavior today**, not an ego-rs guarantee — no workaround
code (Non-Goal: no custom `ConfigurationSource`).

### Decision: widen `build_runtime` return to `Box<dyn std::error::Error>`

**Choice**: `pub fn build_runtime(config: &AppConfig) -> Result<Runtime, Box<dyn std::error::Error>>`.
**Alternatives**: map every failure into `ego_domain::ConfigError` (rejected — mapping boilerplate for an example); a new error enum (rejected — new abstraction, YAGNI, and the proposal forbids inventing error handling).
**Rationale**: The prologue introduces two new error kinds (`kit_config::ConfigError` from `build()`/`load()`, `RuntimeInfraError` from `logging()`/`build_logger()`) alongside the existing `ego_domain::ConfigError` from `validate()`. All three impl `std::error::Error` via thiserror, so `?` auto-boxes with zero mapping. `main.rs`'s `eprintln!("{err}")` and the existing `.is_ok()/.is_err()` tests keep compiling unchanged. `config.validate()?` stays the first line, so the existing validate-first tests still fail exactly where they do today. `Send + Sync` not needed for a single-threaded example.

### Decision: no service-sdk / kit-config changes; AppConfig and kit-config coexist

`ConfigurationProvider`, `build_logger`, `with_logger` are already public
(CORE-017). AppConfig keeps its in-process CORE-016 role; kit-config materializes
only the logging subtree. The two config models coexisting is the scoped point,
not an oversight (Non-Goals: no DB stub, no other typed views).

## Error-Handling Notes (observed kit-config behavior)

| Scenario | Where it surfaces | Result |
|----------|-------------------|--------|
| `config.toml` **missing** | `.load()` (`add_toml` is non-optional) | `kit_config::ConfigError::SourceError` → `?` → `Err` |
| `config.toml` **malformed** | silently swallowed by `TomlFileSource::load` → `logging` key absent | `ConfigurationProvider::logging()` → `ConfigInvalid` → `Err` |
| `logging.format` invalid value | `.logging()` serde | `ConfigInvalid` → `Err` |
| logger `init()` fails | `build_logger` | `LoggerInit` → `Err` |

All route through `?` to the widened return. A malformed file failing *late*
(at the provider, not the loader) is a kit-config quirk worth a one-line comment.

## File Changes

| File | Action | Description |
|------|--------|-------------|
| `examples/reference-app/Cargo.toml` | Modify | Add `kit-config` git dep + `serde_json = "1"` |
| `examples/reference-app/config.toml` | Create | Minimal `[logging]` table the example loads |
| `examples/reference-app/src/lib.rs` | Modify | kit-config prologue in `build_runtime`; widen return type; rewrite stale "out of scope" doc comment; precedence-limitation `// ponytail:` note |
| `examples/reference-app/tests/pipeline.rs` | Modify | Add one test proving the real kit-config path builds end-to-end |

## Testing Strategy

Strict TDD — tasks/apply write these; design only specifies them.

| Layer | What to Test | Approach |
|-------|-------------|----------|
| Integration (new) | `build_runtime(&AppConfig::default())` succeeds with the shipped `config.toml`, exercising the real `ConfigLoader → ConfigurationProvider → build_logger → with_logger` path end-to-end | assert `build_runtime(&config).is_ok()`; red if the TOML is missing, `logging` is malformed, or any boundary call fails |
| Integration (existing) | The three current CORE-016 pipeline tests keep passing — they now *also* traverse the real kit-config path implicitly | run `pipeline.rs` unchanged in intent |

No unit tests added: the boundary types (`ConfigurationProvider`, `build_logger`)
already have unit coverage in service-sdk (CORE-017). This change is composition,
so an integration assertion is the honest boundary. The runtime's logger is not
asserted directly — no public accessor exists on `Runtime` for it from an
external crate, and adding one is a Non-Goal (no service-sdk changes).

**Decision — error paths documented, not tested**: the Error-Handling Notes
table above (missing file, malformed file, invalid `logging.format`, logger
init failure) is deliberately left untested by this change. These are
`kit-config`/`ConfigurationProvider` failure semantics already covered by
their own test suites (CORE-017, kit-config's own tests) — re-asserting them
here would test implementation details of dependencies, not this example's
own logic. `build_runtime`'s only new logic is threading `?` through them,
which the happy-path test already exercises structurally. This is a scoping
choice, not an oversight.

## Migration / Rollout

Single commit, example-only. Revert = restore three files + delete `config.toml`.
No framework API, data, or consumer migration. Well within the 400-line PR budget.

## Open Questions

- [ ] **CI reachability of `pablogore/kit-config`** — same private-repo access
  already proven for kitlogger git deps; confirm at apply, don't assume
  (carried from proposal Risk 1).
- [ ] **Exact current branch** of kit-config to pin in `Cargo.toml` at apply
  time — resolved during apply, deliberately not fixed here.
