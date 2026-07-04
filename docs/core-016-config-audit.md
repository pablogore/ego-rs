# CORE-016 Audit: Configuration Framework vs `kit-config`

Architecture audit determining whether CORE-016 (originally planned as a dedicated Configuration Framework) is still necessary now that `kit-config` exists as a reusable, external configuration crate. Based on direct source reads of `kit-config`'s cached checkout, `git log` across this repo, and grep across the full workspace — not on trusting PRD.md or the roadmap audit's silence on CORE-016.

---

## Key discovery

**`kit-config` is not actually integrated into this repo at all.** It is not a workspace member, not listed in any `Cargo.toml`, and not present in `Cargo.lock`. It exists only as three stray git-cache checkouts (`~/.cargo/git/checkouts/kit-config-*`) fetched at some point but never wired into the workspace. Everything currently labeled "kit-config integration" in `ego-rs` consists of `#[derive(serde::Deserialize)]` on 4 config structs plus 3 hand-written `from_value(serde_json::Value) -> Result<Self, serde_json::Error>` adapter methods — with doc comments *describing* a future `kit_config::ConfigLoader` caller that doesn't exist anywhere in this repository. There is also no application/binary crate in this workspace at all (all 15 members are libraries), so there is currently no *place* to ever call `ConfigLoader::builder()`.

**CORE-016 already shipped**, in four real commits on 2026-06-28 (`#104`–`#108`): built `ego-config-sdk`, correctly identified it as redundant with kit-config, thinned it to a typed accessor, then deleted it entirely. This has **zero OpenSpec trail** — no proposal, spec, design, tasks, or archive-report exists anywhere for CORE-016. Both `PRD.md` and `docs/core-roadmap-audit.md` are stale/wrong about it: PRD.md still lists `ego-config-sdk` as "deferred until service SDK adoption grows" (false — it was built *and removed* the same day), and the roadmap audit's table jumps from CORE-015 straight to CORE-013/CORE-014 discussion without ever mentioning CORE-016's existence or removal.

---

## Review Goals — Answers

**1. Is CORE-016 still required?**
Yes, but not as originally scoped. The current state is an unfinished, undocumented promise: 4 crates carry doc comments describing a `kit_config::ConfigLoader` integration that has never actually been wired into the dependency graph. CORE-016 (or a successor) is needed to close that gap — either by formalizing the pattern already in use, or by explicitly declaring the current state sufficient and stopping the promise of an integration that isn't coming soon.

**2. Should it only specify integration with kit-config?**
Yes — and "integration" here is deliberately small. Add `kit-config` as a real dependency only at a future composition-root/service binary (which doesn't exist in this workspace today), not in any library crate. Formally document the `from_value` adapter pattern already adopted 3 times so future crates follow it consistently instead of re-deriving it from scattered doc comments.

**3. Which previously-planned responsibilities already exist inside kit-config?**
Layered merge (partial/flat only), TOML/YAML/JSON sources, environment variable source, a validation trait, a builder API, and strong typing via serde. This is exactly why the original `ego-config-sdk` (which reimplemented a `ConfigurationProvider`/`ConfigurationSource` pair) was correctly identified as redundant and deleted.

**4. Which responsibilities are still missing?**
These are gaps *inside kit-config itself*, not ego-rs's problem to solve: CLI overrides (absent entirely), a real default-value mechanism (`DefaultsSource` is a no-op stub that always returns `{}`), working profile/environment resolution (`ConfigurationProfile` exists as a bare struct, never wired to `ConfigLoader`), a secrets abstraction (explicit non-goal in kit-config's own spec), hot reload (explicit non-goal), deep/nested merge (current merge is flat, top-level-key overwrite only), and nested environment-variable key mapping (kit-config's own spec claims `EGO_DATABASE_HOST` maps to `database.host`; the code does not implement this transformation anywhere).

**5. Is there duplicated architecture between CORE-016 and kit-config?**
Not currently — the original duplication (`ConfigurationProvider`/`ConfigurationSource` reimplemented in `ego-config-sdk`) was already deleted via the PR1 → PR2 → removal sequence. The only residual near-duplication is the `from_value()` one-line shim repeated verbatim in 3 crates — trivial, arguably fine as-is, not real architecture.

---

## Capability Gap Analysis

| Capability | Already in kit-config | Missing | Keep in CORE-016 |
|---|---|---|---|
| ConfigurationProvider abstraction | ✅ `ConfigurationSource` trait | — | No |
| Layered configuration | ⚠️ Partial — flat, priority-sorted, top-level-key overwrite only, no deep merge | Deep/nested merge | No |
| TOML | ✅ `TomlFileSource` | — | No |
| YAML | ✅ `YamlFileSource` | — | No |
| JSON | ✅ `JsonFileSource` | — | No |
| Environment variables | ⚠️ Partial — flat only; spec claims nested-key mapping, code doesn't implement it | Nested env mapping | No |
| CLI overrides | ❌ Absent — no CLI parser anywhere in kit-config | CLI source | No — belongs to a future composition-root binary, not kit-config or CORE-016 |
| Default values | ⚠️ `DefaultsSource` is a no-op stub (always returns `{}`) | Real defaults mechanism | No — ego-rs crates already solve this locally via `impl Default for XConfig` |
| Validation | ✅ `Validation` trait + 3-tier `ValidationReport`, but no automatic pipeline runner exists despite the spec describing one | Automatic multi-stage pipeline | No |
| Strong typing | ✅ `load_and_validate::<T>()` + serde derives | — | No — this is exactly what the 4 crates already adopted |
| Hot reload | ❌ Explicit non-goal in kit-config's own spec | — | No |
| Profiles/environments | ⚠️ `ConfigurationProfile` type exists, never wired to `ConfigLoader` | Profile-driven source selection | No |
| Secrets abstraction | ❌ Explicit non-goal; cloud sources (AWS/GCP/DigitalOcean) pass credentials as plaintext `Value::String`, no redaction, no zeroize | Everything | **Move out** — separate Secrets change |
| Merge strategy | ⚠️ Same limitation as "layered configuration" above | Deep merge | No |
| Builder API | ✅ `ConfigLoaderBuilder` fluent chain | — | No |

---

## Integration Audit

`ego-rs` should **not** expose a `use kit_config::...` wrapper crate, and should **not** build an adapter layer. The library crates already converged on the right minimal shape through their own history: each defines a `#[derive(serde::Deserialize)] pub struct XConfig` with `impl Default`, and exposes a `from_value(value: serde_json::Value) -> Result<Self, serde_json::Error>` entry point — with zero direct dependency on kit-config. This decouples every library crate from the loader entirely; only a future composition-root binary (which doesn't exist in this workspace yet, since all 15 members are libraries) would ever add `kit-config` as a real dependency and call `ConfigLoader::builder()...build()?.load()`, dispatching `Value` slices to each crate's `from_value`.

An adapter/wrapper crate was tried once — `ego-config-sdk`, built as a "thin typed accessor over kit-config," then deleted a commit later once it was recognized as unnecessary. Building a new one now would repeat a mistake this codebase already corrected.

---

## Architectural Duplication Found

- **`ConfigurationProvider` / `ConfigBuilder` / `ConfigSource` / `ConfigResolver`**: zero hits anywhere in the current `ego-rs` tree. These existed only in the now-deleted `ego-config-sdk`.
- **`ConfigLoader`**: no type definition in `ego-rs` — all references are doc comments pointing at the (non-vendored) `kit_config::ConfigLoader`.
- **`ConfigValue`**: one live definition, `service-sdk::di::ConfigValue<T>` — a generic DI wrapper, unrelated to configuration loading despite the name. A second, differently-shaped `ConfigValue` (a dynamically-typed scalar enum) existed in the now-deleted `config-sdk` and is gone.
- **`EventBusConfig` vs `SchedulerEventBusConfig`** (found during this audit, unrelated to kit-config): two independently-defined event-bus-capacity configs in different crates (`ego-scheduler` and `persistent-entity`), same domain, no shared type, no conversion between them.

---

## Recommended Architecture (minimal)

- No new configuration crate in `ego-rs`. No adapter/wrapper crate.
- Library crates keep doing what 3 of them already do: `Deserialize` struct + `Default` impl + `from_value` one-liner, zero dependency on kit-config.
- The only place that would ever add a real `kit-config` dependency is a future composition-root binary — not speculatively built now.
- Do not chase kit-config's own gaps (nested env mapping, real defaults, profiles, CLI). None of them block `ego-rs` today, since no crate is actually loading configuration through it yet.

---

## Proposed Scope for CORE-016

1. One functional requirement: any `ego-rs` crate with runtime-tunable configuration MUST expose a `Deserialize` struct + `from_value`, and MUST NOT depend on kit-config (or any loader crate) directly.
2. Document the `from_value` convention as a named, first-class pattern — currently it's only repeated doc-comment prose in 3 files, not a spec requirement.
3. Retroactively archive the already-shipped work (commits `#104`–`#108`) with a real proposal/spec/archive-report, closing the paper-trail gap for a change that already happened without one.
4. Record explicitly as an Open Question: "no composition root exists yet to actually invoke `kit_config::ConfigLoader`" — deferred, not a blocker.
5. Explicitly state: no `ego-rs`-side reimplementation of anything kit-config already owns.

---

## What Should Move to Another CORE Change

- **Secrets abstraction** (Vault, AWS Secrets Manager, Azure Key Vault, GCP Secret Manager): its own change — not kit-config (explicit non-goal there) and not CORE-016. `security-apikey`'s `zeroize`-based standard is already stricter than kit-config's plaintext cloud-credential handling; a future Secrets change should match that bar, not kit-config's.
- **CLI overrides**: whatever future composition-root/CLI crate gets built, if ever.
- **`EventBusConfig` vs `SchedulerEventBusConfig` duplication**: worth its own small cleanup ticket, unrelated to CORE-016.
- **Telemetry/Logging framework**: confirmed nonexistent (CORE-012 is mislabeled per the earlier roadmap audit — the number was reused for Security Context Unification, and no logging framework exists). kit-config already has a fairly complete `LoggingConfig` model (`config-models/logging.rs`) that could seed a future logging change, but that is its own CORE item.

**Future secrets providers (Vault, AWS, Azure, GCP)** belong to **(B) a Secrets framework**, separate from kit-config entirely — not (A) kit-config (its spec explicitly excludes this) and not yet (C) per-vendor integration crates (building those before a Secrets trait exists would repeat the exact "providers before framework" pattern this codebase's own `config-sdk` history already self-corrected once).
