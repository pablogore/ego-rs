//! `cargo metadata` invocation and parsing into the plain `Graph` +
//! crate-name shape `layers`/`cycles` operate on (design.md AD-2).
//!
//! Scope note: every check in this tool is restricted to packages whose
//! manifest lives under `<workspace_root>/crates/`. This excludes both
//! `examples/reference-app` (composition-root binary, explicitly out of
//! completeness scope per design.md §2) and `xtask` itself (the tool's own
//! package, which lives at the workspace root, not under `crates/`) from
//! all three checks, keeping the "16 crates" contract exact.

use crate::layers::Graph;
use std::collections::HashMap;
use std::path::Path;

#[derive(serde::Deserialize)]
struct RawMetadata {
    packages: Vec<RawPackage>,
    workspace_members: Vec<String>,
    workspace_root: String,
    resolve: RawResolve,
}

#[derive(serde::Deserialize)]
struct RawPackage {
    id: String,
    name: String,
    manifest_path: String,
}

#[derive(serde::Deserialize)]
struct RawResolve {
    nodes: Vec<RawNode>,
}

#[derive(serde::Deserialize)]
struct RawNode {
    id: String,
    deps: Vec<RawDep>,
}

#[derive(serde::Deserialize)]
struct RawDep {
    pkg: String,
    dep_kinds: Vec<RawDepKind>,
}

#[derive(serde::Deserialize)]
struct RawDepKind {
    kind: Option<String>,
}

pub struct Workspace {
    /// Normal + build dependency edges among `crates/*` workspace members.
    pub graph: Graph,
    /// All `crates/*` workspace member names, sorted.
    pub crates: Vec<String>,
}

/// Runs `cargo metadata --format-version 1` and builds a [`Workspace`] from
/// its output.
pub fn load_workspace() -> anyhow::Result<Workspace> {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string());
    let output = std::process::Command::new(cargo)
        .args(["metadata", "--format-version", "1"])
        .output()
        .map_err(|e| anyhow::anyhow!("running cargo metadata: {e}"))?;
    if !output.status.success() {
        anyhow::bail!(
            "cargo metadata failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let raw: RawMetadata = serde_json::from_slice(&output.stdout)
        .map_err(|e| anyhow::anyhow!("parsing cargo metadata output: {e}"))?;
    Ok(build_workspace(&raw))
}

/// Whether `manifest_path` lives under `<workspace_root>/crates/`, compared
/// as paths (not strings) so a `cargo metadata`-reported path using the
/// platform's native separator (e.g. `\` on Windows) is handled correctly —
/// a hardcoded `/`-joined string prefix would silently match nothing there.
fn is_under_crates_dir(manifest_path: &str, workspace_root: &str) -> bool {
    let crates_dir = Path::new(workspace_root).join("crates");
    Path::new(manifest_path).starts_with(&crates_dir)
}

fn build_workspace(raw: &RawMetadata) -> Workspace {
    let members: std::collections::HashSet<&str> =
        raw.workspace_members.iter().map(String::as_str).collect();

    let id_to_name: HashMap<&str, &str> = raw
        .packages
        .iter()
        .map(|p| (p.id.as_str(), p.name.as_str()))
        .collect();

    let crate_ids: Vec<&str> = raw
        .packages
        .iter()
        .filter(|p| members.contains(p.id.as_str()) && is_under_crates_dir(&p.manifest_path, &raw.workspace_root))
        .map(|p| p.id.as_str())
        .collect();
    let crate_id_set: std::collections::HashSet<&str> = crate_ids.iter().copied().collect();

    let mut graph: Graph = crate_ids
        .iter()
        .map(|id| (id_to_name[id].to_string(), Vec::new()))
        .collect();

    for node in &raw.resolve.nodes {
        if !crate_id_set.contains(node.id.as_str()) {
            continue;
        }
        let from_name = id_to_name[node.id.as_str()].to_string();
        for dep in &node.deps {
            if !crate_id_set.contains(dep.pkg.as_str()) {
                continue;
            }
            let is_normal_or_build = dep
                .dep_kinds
                .iter()
                .any(|k| matches!(k.kind.as_deref(), None | Some("build")));
            if !is_normal_or_build {
                continue;
            }
            graph
                .entry(from_name.clone())
                .or_default()
                .push(id_to_name[dep.pkg.as_str()].to_string());
        }
    }

    let mut crates: Vec<String> = crate_ids.iter().map(|id| id_to_name[id].to_string()).collect();
    crates.sort();

    Workspace { graph, crates }
}

/// Resolves the workspace root from `xtask`'s own manifest location (a
/// direct workspace-root member), avoiding a second `cargo metadata` call
/// just to find a path already implied by `CARGO_MANIFEST_DIR`.
pub fn workspace_root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask/Cargo.toml has a parent directory (the workspace root)")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `a` depends on `b` (dev), `c` (normal, `kind: null`), and `d` (build).
    /// Only the normal and build edges should survive into the graph — the
    /// dev edge (e.g. the legitimate `service-sdk <-> testkit` cycle) must
    /// not appear, since dev-deps aren't part of the build graph the layer
    /// checks reason about.
    fn fixture_json() -> &'static str {
        r#"{
            "packages": [
                {"id": "a 0.1.0", "name": "a", "manifest_path": "/ws/crates/a/Cargo.toml"},
                {"id": "b 0.1.0", "name": "b", "manifest_path": "/ws/crates/b/Cargo.toml"},
                {"id": "c 0.1.0", "name": "c", "manifest_path": "/ws/crates/c/Cargo.toml"},
                {"id": "d 0.1.0", "name": "d", "manifest_path": "/ws/crates/d/Cargo.toml"}
            ],
            "workspace_members": ["a 0.1.0", "b 0.1.0", "c 0.1.0", "d 0.1.0"],
            "workspace_root": "/ws",
            "resolve": {
                "nodes": [
                    {"id": "a 0.1.0", "deps": [
                        {"pkg": "b 0.1.0", "dep_kinds": [{"kind": "dev"}]},
                        {"pkg": "c 0.1.0", "dep_kinds": [{"kind": null}]},
                        {"pkg": "d 0.1.0", "dep_kinds": [{"kind": "build"}]}
                    ]},
                    {"id": "b 0.1.0", "deps": []},
                    {"id": "c 0.1.0", "deps": []},
                    {"id": "d 0.1.0", "deps": []}
                ]
            }
        }"#
    }

    #[test]
    fn dev_dependency_excluded_normal_and_build_included() {
        let raw: RawMetadata = serde_json::from_str(fixture_json()).unwrap();
        let workspace = build_workspace(&raw);

        let a_deps = &workspace.graph["a"];
        assert!(!a_deps.contains(&"b".to_string()), "dev dep must be excluded");
        assert!(a_deps.contains(&"c".to_string()), "normal (kind: null) dep must be included");
        assert!(a_deps.contains(&"d".to_string()), "build dep must be included");
    }

    #[test]
    fn discovers_crates_with_platform_native_paths() {
        // Path::starts_with compares components, not raw bytes, so this must
        // hold regardless of how workspace_root's trailing separator (or
        // lack of one) is spelled.
        assert!(is_under_crates_dir("/ws/crates/domain/Cargo.toml", "/ws"));
        assert!(is_under_crates_dir("/ws/crates/domain/Cargo.toml", "/ws/"));
        assert!(!is_under_crates_dir(
            "/ws/examples/reference-app/Cargo.toml",
            "/ws"
        ));
        assert!(!is_under_crates_dir("/ws/xtask/Cargo.toml", "/ws"));
    }
}
