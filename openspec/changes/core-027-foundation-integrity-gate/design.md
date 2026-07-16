# Design: CORE-027 — Foundation Integrity Gate

## 1. Technical Approach

Close the gap between documented and enforced architecture with one runnable
tool and one corrected data file. Order matters: **fix `layers.toml` first**
(complete the map, remove the dead `runtime-slice` entry), **then build the
checker against it**, so the tool is green on day one and every later failure
is a real regression. The checker is a single Rust `xtask` binary with three
subcommands (`verify-layers`, `verify-isolation`, `verify-hygiene`) driven off
`cargo metadata` JSON, plus an evidence-driven flaky-test triage. No CI, no
Dagger, no new functional capability (proposal §3).

## 2. Layer Assignments (Decision 1)

Existing vocabulary is `domain / application / infrastructure / transport`
plus `foundation` (`ego-runtime`, `ego-scheduler`). Three tiers are added
because three crates genuinely do not fit the four documented layers — each is
grounded in `ARCHITECTURE.md`, not invented here.

| Crate (package) | Layer | Real deps | Rationale |
|---|---|---|---|
| `ego-persistence` | infrastructure | `ego-domain` (+sqlx) | Concrete Postgres persistence adapter over domain ports. |
| `ego-event-adapter` | infrastructure | `ego-domain` | Event-conversion adapter; adapter = infrastructure concern. |
| `persistent-entity` | foundation | `ego-domain` | Actor/entity runtime primitive; `ego-runtime` (foundation) depends on it, so it cannot sit above foundation. |
| `ego-security-sdk` | cross-cutting *(new)* | `ego-domain` | `ARCHITECTURE.md` names it the one genuinely cross-cutting crate: security ports (`AuthenticationProvider`, `SecurityContext`) consumed by infra, transport, and the SDK. |
| `security-apikey` | infrastructure | `ego-domain`, `ego-security-sdk` | Concrete API-key auth adapter (mirrors `security-jwt`). |
| `ego-service-sdk` | sdk *(new)* | `ego-domain`, `ego-security-sdk`, `ego-runtime`, `persistent-entity` | Service-building framework. Cannot be `infrastructure` (would break `transport ↛ infrastructure`); cannot be `application` (would force the pure application rule to permit foundation deps). Earns its own tier. |
| `ego-service-sdk-macros` | tooling *(new)* | none (syn/quote only) | Compile-time proc-macro; no production crate depends on it (dev-dep only). |
| `ego-testkit` | tooling *(new)* | domain, runtime, security-sdk, service-sdk, persistent-entity | Test-support doubles; depends *upward* on the SDK, consumed only as `[dev-dependencies]`. |

**Allowed-dependency matrix** (checker rules; `→` = may depend on):

| Layer | May depend on |
|---|---|
| domain | — |
| foundation | domain, foundation |
| cross-cutting | domain |
| application | domain |
| infrastructure | domain, application, foundation, cross-cutting, infrastructure |
| sdk | domain, foundation, cross-cutting |
| transport | domain, application, cross-cutting, sdk |
| tooling | any (sink — no production crate may depend on tooling) |

Invariants preserved from the original header: `transport ↛ infrastructure`,
`infrastructure ↛ transport`, `application → domain only`, `domain → nothing`.
`cross-cutting` shares `foundation`'s rule shape but is kept distinct to honor
`ARCHITECTURE.md`'s explicit "cross-cutting" classification. `examples/*`
(`reference-app`) is out of completeness scope: a composition-root binary may
depend on any layer and nothing depends on it. This keeps the "16 crates"
contract exact.

## 3. Architecture Decisions

| AD | Decision | Rejected | Rationale |
|---|---|---|---|
| **AD-1 Tool form** (Dec. 2) | Rust `xtask` workspace crate. | `scripts/verify-layers.sh` + `jq`. | Cycle detection is a graph algorithm and TOML+JSON parsing from bash is fragile and needs a new `jq` system dep. Rust reuses `serde_json`/`toml` (already ubiquitous), is cross-platform, unit-testable, and correct on edge cases. `cargo run -p xtask -- verify-layers` is trivially Dagger-callable. Cost: `xtask` joins the graph — mapped `tooling`. |
| **AD-2 Graph source** (Dec. 3) | `cargo metadata --format-version 1`; iterate `workspace_members`, take each package's **normal + build** `dependencies` whose name is also a workspace member. | Hand-parsing `Cargo.toml`. | Metadata is the resolved truth (path deps, renames, features). **Dev-dependencies are excluded** so the legitimate `service-sdk ↔ testkit` dev-dep cycle is not flagged (Cargo allows dev-dep cycles). |
| **AD-3 Checks & exit** (Dec. 3) | `verify-layers` runs three checks: (a) **direction** — every edge `A→B` fails unless `layer(B) ∈ allowed[layer(A)]`; (b) **cycles** — Tarjan SCC over the normal-dep graph, any SCC size >1 fails; (c) **completeness** — every `crates/*` member has a `layers.toml` entry AND every entry maps to a real package (catches dead `runtime-slice`). Exit `0` pass / `1` any failure; report groups violations by class. | Distinct exit code per class; first-failure abort. | One report listing all violations is more useful than early exit. `1` is enough for a pipeline gate. |
| **AD-4 Isolation** (Dec. 4) | `verify-isolation` subcommand loops `cargo check -p <crate> --no-default-features` over every `crates/*` member. | Separate script; `cargo check --workspace`. | Per-`-p` resolve builds each crate's own subtree, defeating workspace feature-unification that could mask a feature gate a narrow downstream consumer needs. No crate declares a `default` feature, so `--no-default-features` is a harmless strict floor. Same tool = consolidation (proposal §3.4). |
| **AD-5 Hygiene** (Dec. 6) | `verify-hygiene` subcommand: for each dir under `openspec/changes/` (excluding `archive/`), strip the `YYYY-MM-DD-` prefix from every `archive/*` name and fail on a case-insensitive suffix match. | Separate script. | Mechanically detects an un-archived duplicate of an archived change (e.g. active `core-019-…` vs archived `2026-07-15-core-019-…`). Same-tool consolidation. |

## 4. Flaky-Test Triage Methodology (Decision 5)

Three suspects, exact names pinned against source:

| Suspect | Test(s) |
|---|---|
| persistent-entity concurrent spawn | `registry.rs::concurrent_lookups_for_one_triple_spawn_exactly_once`; `mailbox.rs::close_and_drain_races_concurrent_sends_without_losing_envelopes` (the two adversarial tests fixed by `2d5861d`) |
| effects deadline/cancellation | `effects/acceptor.rs::acceptance_in_progress_is_cancelled_once_the_deadline_instant_actually_elapses`; `::lost_wakeup_pattern_is_reproduced_with_a_widened_race_window`; `effects/runner.rs::shutdown_reaches_drain_deadline_despite_a_hung_backpressure_permit_wait` |
| provider-access under parallel execution | the `#[tokio::test]` module in `crates/runtime/src/providers/access.rs` |

**Protocol per suspect** (evidence-driven, local — CI-like load reproduced by
contention): `N = 200` tight-loop runs
(`for i in $(seq 1 200); do cargo test -p <crate> <test> -- --exact || break; done`)
**plus** ~50 full-crate parallel sweeps
(`cargo test -p <crate> -- --test-threads=$(( $(sysctl -n hw.ncpu) * 4 ))`) to
force scheduler interleaving. **Verdict** per suspect ∈ {`fixed` (root cause +
fix commit), `non-reproducing` (N clean runs, evidence noted)}. Verdicts are
recorded in `openspec/changes/core-027-foundation-integrity-gate/flaky-triage.md`
(apply phase) and summarized in `verify-report.md`. A fix that changes
spec-level behavior triggers a spec delta (proposal §4).

## 5. File Changes

| File | Action | Description |
|---|---|---|
| `layers.toml` | Modify | Drop `runtime-slice`; add the 8 crates; add `cross-cutting`/`sdk`/`tooling` to the allowed-layers header; name the real tool. |
| `xtask/Cargo.toml`, `xtask/src/main.rs` | Create | Checker: `verify-layers` / `verify-isolation` / `verify-hygiene`. |
| `Cargo.toml` (workspace) | Modify | Add `xtask` to `members`. |
| `openspec/changes/.../flaky-triage.md` | Create | Per-suspect verdict + run evidence. |
| `crates/persistent-entity`, `crates/runtime` (effects/providers) | Conditional | Only if a suspect still reproduces. |
| `openspec/specs/foundation-integrity/` | Create | Canonical spec (spec phase). |

## 6. Testing Strategy

| Layer | What | Approach |
|---|---|---|
| Unit | Direction rule, Tarjan SCC, completeness both-ways | `xtask` `#[test]` over synthetic layer maps + fixture graphs |
| Integration | Checker fails on each class deliberately injected (wrong edge, cycle, unmapped crate); passes on fixed workspace | Run `xtask` against the real graph + temporary violations |
| Manual | Isolation sweep green; hygiene green | `cargo run -p xtask -- verify-isolation \| verify-hygiene` |
| Triage | Three suspects | §4 protocol, verdict recorded |

## 7. Threat Matrix

`verify-isolation` shells out to `cargo check`; `verify-hygiene` reads
`openspec/changes/` paths. Both invoke only fixed, hard-coded argv (no
user/network input), operate read-only, and run locally.

| Row | Applicable | Behavior |
|---|---|---|
| Command injection via argv | N/A | Crate names come from `cargo metadata` (trusted), never interpolated shell strings; `Command` args passed as a vector. |
| Path traversal | Applicable | Directory scan is confined to the repo root resolved from `cargo metadata`'s `workspace_root`; symlinks not followed. |
| Untrusted process output | N/A | Only this repo's own `cargo` is invoked; no PR/VCS automation, no executable-file classification. |

## 8. Migration / Rollout

No migration. Everything is additive tooling plus a data-file fix. Rollback:
revert the commit range to restore the old `layers.toml` and delete `xtask/`;
flaky fixes (if any) are independent, individually revertible commits. No
runtime behavior, public API, or persisted data is touched.

## 9. Open Questions

- [ ] Whether `cross-cutting` and `sdk` should later collapse if the graph
      simplifies — settled as distinct now; revisit only on a real merge signal.
- [ ] Exact `provider-access` test name to loop is pinned at apply time by
      grepping `#[tokio::test]` in `access.rs` (module identified; single name
      not asserted here to avoid guessing).
