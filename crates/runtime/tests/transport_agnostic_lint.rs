//! CORE-019 Phase 10 (10.1 RED / 10.2 GREEN) — transport-agnosticism lint.
//!
//! Proposal success criterion §20: "No `match` on `effect_type`/`destination`
//! string literals anywhere in `crates/runtime`/`crates/service-sdk`
//! (registry only)." Spec: "Runtime Remains Transport-Agnostic" — "the
//! runtime and service-sdk crates implementing this capability" must have
//! "none" outside the executor registry lookup itself.
//!
//! # Best-effort text scan (same spirit as `service-sdk`'s
//! `tenant_scoped_lint.rs` FR-007 check)
//!
//! This is a line-based scan, not a full AST/control-flow analysis: it flags
//! any line containing the `match` keyword together with `effect_type` or
//! `destination`, which is the shape any accidental
//! `match effect_type.as_str() { "http" => ... }`-style branch would take.
//! `registry.rs`'s `HashMap<String, Arc<dyn ExternalEffectExecutor>>` lookup
//! is the one sanctioned `effect_type`-keyed dispatch point and is excluded
//! by filename, exactly as the spec's own scenario names it ("none exists
//! outside the executor registry lookup itself"). A more deeply nested
//! `match` (e.g. spanning multiple lines with the scrutinee on one line and
//! `effect_type` referenced only inside an arm several lines later) is a
//! known blind spot of this heuristic, same accepted tradeoff as
//! `tenant_scoped_lint.rs` documents for its own identifier scan.
//!
//! Run with: `cargo test -p ego-runtime transport_agnostic_lint`

use std::path::{Path, PathBuf};

/// Ascends from `start` until it finds the ancestor whose `Cargo.toml`
/// declares a `[workspace]` table (same resolution `tenant_scoped_lint.rs`
/// uses, needed because `cargo test`'s CWD is the crate root, not the
/// workspace root).
fn workspace_root(start: &Path) -> PathBuf {
    let mut dir = start.to_path_buf();
    loop {
        let candidate = dir.join("Cargo.toml");
        if candidate.is_file() {
            if let Ok(content) = std::fs::read_to_string(&candidate) {
                if content.contains("[workspace]") {
                    return dir;
                }
            }
        }
        if !dir.pop() {
            panic!(
                "transport_agnostic_lint: could not locate workspace root ascending from {}",
                start.display()
            );
        }
    }
}

/// Recursively collects every `.rs` file under `dir`, excluding
/// `registry.rs` (the one sanctioned `effect_type`-keyed lookup).
fn collect_rs_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_rs_files(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "rs")
            && path.file_name().and_then(|n| n.to_str()) != Some("registry.rs")
        {
            out.push(path);
        }
    }
}

struct Violation {
    location: String,
    line_text: String,
}

fn find_violations(root: &Path, files: &[PathBuf]) -> Vec<Violation> {
    let mut violations = Vec::new();
    for file in files {
        let Ok(content) = std::fs::read_to_string(file) else {
            continue;
        };
        let label = file
            .strip_prefix(root)
            .unwrap_or(file)
            .display()
            .to_string();
        for (line_no, line) in content.lines().enumerate() {
            let has_match = line.contains("match ") || line.trim_start().starts_with("match");
            let mentions_target = line.contains("effect_type") || line.contains("destination");
            if has_match && mentions_target {
                violations.push(Violation {
                    location: format!("{label}:{}", line_no + 1),
                    line_text: line.trim().to_string(),
                });
            }
        }
    }
    violations
}

#[test]
fn no_match_on_effect_type_or_destination_outside_the_registry() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let root = workspace_root(&manifest_dir);

    let scan_dirs = [
        root.join("crates/runtime/src"),
        root.join("crates/service-sdk/src"),
    ];

    let mut files = Vec::new();
    let mut found_any_dir = false;
    for dir in &scan_dirs {
        if dir.is_dir() {
            found_any_dir = true;
            collect_rs_files(dir, &mut files);
        }
    }
    assert!(
        found_any_dir,
        "neither crates/runtime/src nor crates/service-sdk/src was found under {} — \
         the scan is silently scanning zero files",
        root.display()
    );
    assert!(
        !files.is_empty(),
        "workspace scan found no .rs files — CWD/workspace-root resolution is broken"
    );

    let violations = find_violations(&root, &files);

    assert!(
        violations.is_empty(),
        "found a `match` branching on `effect_type`/`destination` outside the executor \
         registry lookup (proposal §20 / spec: \"Runtime Remains Transport-Agnostic\"):\n{}",
        violations
            .iter()
            .map(|v| format!("{}: {}", v.location, v.line_text))
            .collect::<Vec<_>>()
            .join("\n")
    );
}

/// Regression control: `registry.rs` itself performs a `HashMap::get`
/// keyed by `effect_type` (not a `match`), so it must never itself be
/// flagged even though it obviously "mentions" `effect_type` throughout.
#[test]
fn registry_rs_is_excluded_from_the_scan() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let root = workspace_root(&manifest_dir);
    let mut files = Vec::new();
    collect_rs_files(&root.join("crates/runtime/src"), &mut files);

    assert!(
        !files
            .iter()
            .any(|f| f.file_name().and_then(|n| n.to_str()) == Some("registry.rs")),
        "registry.rs must be excluded from the scan by filename"
    );
}
