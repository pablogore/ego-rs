# xtask — Foundation Integrity Gate

Local checker for the workspace's documented layer architecture (see
`../layers.toml` and `../ARCHITECTURE.md`). Part of
[CORE-027](../openspec/changes/core-027-foundation-integrity-gate/proposal.md).
Not wired into CI yet — run manually until the Dagger pipeline picks it up.

## Usage

```sh
cargo run -p xtask -- verify-layers
cargo run -p xtask -- verify-isolation
cargo run -p xtask -- verify-hygiene
```

Each subcommand exits `0` on a clean pass and `1` if it finds any violation,
printing a human-readable report to stdout either way.

## Subcommands

- **`verify-layers`** — parses `layers.toml` and the real dependency graph
  (via `cargo metadata`, normal + build deps only) and fails on:
  - a dependency pointing the wrong direction per the layer rules in
    `layers.toml`'s header;
  - a dependency cycle (Tarjan SCC);
  - a workspace crate missing from `layers.toml`, or a `layers.toml` entry
    naming a crate that doesn't exist, or mapped to an unknown layer name.
- **`verify-isolation`** — runs `cargo check -p <crate> --no-default-features`
  for every `crates/*` member, so workspace feature unification can't hide a
  crate that only compiles because another crate's features are pulled in.
- **`verify-hygiene`** — fails if `openspec/changes/` contains an un-archived
  duplicate of a change already present under `openspec/changes/archive/`.

## Scope

Every check is restricted to packages whose manifest lives under
`<workspace_root>/crates/`. This deliberately excludes `examples/reference-app`
(a composition-root binary that may depend on anything) and `xtask` itself
(lives at the workspace root, not under `crates/`) from all three checks.
