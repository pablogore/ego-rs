# Tasks: CORE-018a — Real kit-config Host Example in reference-app

## Review Workload Forecast

| Field | Value |
|-------|-------|
| Estimated changed lines | ~90-150 (1 dep line, 1 new config file, ~30-line lib.rs prologue + doc rewrite, 1 new test) |
| 400-line budget risk | Low |
| Chained PRs recommended | No |
| Suggested split | Single PR — example-crate wiring only, no framework fan-out |
| Delivery strategy | ask-on-risk |
| Chain strategy | n/a |

Decision needed before apply: No
Chained PRs recommended: No
400-line budget risk: Low

### Suggested Work Units

| Unit | Goal | Likely PR | Notes |
|------|------|-----------|-------|
| 1 | Single PR | PR 1 | One dependency line, one crate's wiring, one config file (proposal Rollback Plan) — splitting adds no isolation value. |

---

## Phase 1: Dependency Setup (`examples/reference-app/Cargo.toml`)

- [x] TASK-001 Resolve the current default/tracking branch of `github.com/pablogore/kit-config` (mirror the `branch = "develop"` pattern already used for kitlogger in `crates/service-sdk/Cargo.toml:14` — confirm kit-config uses the same convention, don't assume). Add `kit-config = { git = "https://github.com/pablogore/kit-config.git", branch = "<resolved>" }` (default features) and `serde_json = "1"` to `examples/reference-app/Cargo.toml`. (Design: "git dependency, default features, no branch pin in the design"; Proposal Risk 1: confirm CI reachability) — Verify: `cargo build -p reference-app` fetches and compiles the new dependency.

## Phase 2: RED — Wire the Prologue Before the Config File Exists

- [x] TASK-002 In `examples/reference-app/src/lib.rs`, add the config-materialization prologue to `build_runtime`: `kit_config::ConfigLoader::builder().add_defaults().add_environment().add_toml(concat!(env!("CARGO_MANIFEST_DIR"), "/config.toml")).build()?.load()?` → `serde_json::to_value(map)?` → `ConfigurationProvider::from_value(value).logging()?` → `build_logger(&settings)?` → conditionally `.with_logger(logger)` on `RuntimeBuilder` before `.build()`. Widen `build_runtime`'s signature to `pub fn build_runtime(config: &AppConfig) -> Result<Runtime, Box<dyn std::error::Error>>`. Do NOT create `config.toml` yet. (Spec: "Reference Host Example Materializes Configuration Through kit-config"; Design Implementation Sketch) — Verify: `cargo build -p reference-app` compiles (types check even though the file path doesn't exist yet — that's a runtime failure, not compile-time).
- [x] TASK-003 Add one integration test to `examples/reference-app/tests/pipeline.rs` asserting `build_runtime(&AppConfig::default()).is_ok()` exercises the real kit-config path end-to-end. (Spec scenario: "build_runtime wires real kit-config output"; Design Testing Strategy) — Verify (RED): `cargo test --workspace -p reference-app` — this new test AND the three existing CORE-016 pipeline tests that assert `.is_ok()` now fail, because `.add_toml(...)` is non-optional and `config.toml` does not exist yet (`kit_config::ConfigError::SourceError` → `?` → `Err`). Confirm all failures are this one root cause, not a compile error or unrelated regression.

## Phase 3: GREEN — Ship the Config File

- [x] TASK-004 Create `examples/reference-app/config.toml` with the minimal `[logging]` table: `enabled = true`, `format = "pretty"`. (Design: "TOML anchored to `CARGO_MANIFEST_DIR`, not CWD-relative") — Verify (GREEN): `cargo test --workspace -p reference-app` — the new test from TASK-003 and all three existing CORE-016 pipeline tests pass again, now traversing the real `ConfigLoader → ConfigurationProvider → build_logger → with_logger` path.

## Phase 4: Documentation Cleanup

- [x] TASK-005 In `examples/reference-app/src/lib.rs`, rewrite the stale module doc comment ("kit-config ... is intentionally out of scope ...") to describe the real kit-config wiring now in place. (Proposal Success Criteria: "Stale 'out of scope' doc comment is gone") — Verify: no remaining occurrence of "intentionally out of scope" (`rg "intentionally out of scope" examples/reference-app/src/lib.rs` returns nothing).
- [x] TASK-006 Add a `// ponytail:` comment above `.add_environment()` in the prologue documenting the observed (not guaranteed) kit-config precedence: file sources (priority 200) outrank environment sources (priority 50), and env vars cannot populate or override the nested `logging` object. (Design Decision: "keep `add_environment()`, document the flat-source limitation in code"; Proposal: precedence documentation) — Verify: the comment accurately reflects the current behavior of kit-config observed at apply time (file sources outrank environment sources; env cannot override nested `logging`) — do not pin the verification to specific priority numbers, since those are internal `kit-config` implementation details, not a contract.

## Phase 5: Full Verification

- [x] TASK-007 Run `cargo test --workspace` and confirm: all `reference-app` tests pass (3 existing + 1 new), `crates/service-sdk` and `crates/service-sdk/examples/logging_bootstrap.rs` show zero diff (`git diff --stat crates/service-sdk`), and `main.rs`'s `eprintln!("{err}")` still compiles unchanged against the widened `Box<dyn std::error::Error>` return type. (Spec scenario: "Existing framework contract remains untouched"; Proposal Success Criteria) — Verify: clean `cargo test --workspace` run, empty `git diff` for `crates/service-sdk/`.

## Explicitly Not Tested (per Design decision — do not add tasks for these)

Missing `config.toml`, malformed TOML, invalid `logging.format`, and logger `init()` failure are documented in Design's Error-Handling Notes table but deliberately left untested here — they are `kit-config`/`ConfigurationProvider` failure semantics already covered by their own test suites (CORE-017, kit-config's tests).
