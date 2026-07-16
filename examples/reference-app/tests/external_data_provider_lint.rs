//! CORE-019A Phase 6 (6.3) — external-data-provider construction lint.
//!
//! Mirrors `ego-runtime`'s `transport_agnostic_lint.rs` (CORE-019 Phase 10):
//! a best-effort, line-based scan proving no `PersistentEntity` handler in
//! `examples/reference-app` constructs the dogfood provider (or any other
//! `ExternalDataProvider` implementation) directly — every fetch path routes
//! through the registered `DataProviderAccess` facade
//! (`external-data-providers` spec: "Reference-app handler never constructs
//! a client inline"; `persistent-entity` spec: "Handler fetches external
//! data during command handling").
//!
//! `domain/pricing.rs` is excluded by filename — it is the one sanctioned
//! definition site for `PricingLookupProvider` itself, exactly as
//! `registry.rs` is excluded from CORE-019's transport lint.

use std::path::{Path, PathBuf};

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
                "external_data_provider_lint: could not locate workspace root ascending from {}",
                start.display()
            );
        }
    }
}

/// Recursively collects every `.rs` file under `dir`, excluding
/// `pricing.rs` (the one sanctioned definition site for the dogfood
/// provider).
fn collect_rs_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_rs_files(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "rs")
            && path.file_name().and_then(|n| n.to_str()) != Some("pricing.rs")
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
        let label = file.strip_prefix(root).unwrap_or(file).display().to_string();
        for (line_no, line) in content.lines().enumerate() {
            if line.contains("PricingLookupProvider") || line.contains("impl ExternalDataProvider") {
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
fn no_handler_outside_pricing_rs_constructs_the_dogfood_provider_directly() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let root = workspace_root(&manifest_dir);

    let scan_dir = root.join("examples/reference-app/src");
    assert!(
        scan_dir.is_dir(),
        "examples/reference-app/src was not found under {} — the scan is silently scanning zero files",
        root.display()
    );

    let mut files = Vec::new();
    collect_rs_files(&scan_dir, &mut files);
    assert!(!files.is_empty(), "workspace scan found no .rs files — CWD/workspace-root resolution is broken");

    let violations = find_violations(&root, &files);

    assert!(
        violations.is_empty(),
        "found a reference-app handler constructing the dogfood provider (or a second \
         ExternalDataProvider impl) outside domain/pricing.rs — every fetch path must route \
         through the registered DataProviderAccess facade instead:\n{}",
        violations
            .iter()
            .map(|v| format!("{}: {}", v.location, v.line_text))
            .collect::<Vec<_>>()
            .join("\n")
    );
}

/// Regression control: `pricing.rs` itself defines `PricingLookupProvider`
/// and its `impl ExternalDataProvider`, so it must never itself be flagged
/// even though it obviously contains both patterns.
#[test]
fn pricing_rs_is_excluded_from_the_scan() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let root = workspace_root(&manifest_dir);
    let mut files = Vec::new();
    collect_rs_files(&root.join("examples/reference-app/src"), &mut files);

    assert!(
        !files.iter().any(|f| f.file_name().and_then(|n| n.to_str()) == Some("pricing.rs")),
        "pricing.rs must be excluded from the scan by filename"
    );
}
