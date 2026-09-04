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
//! `providers/pricing_lookup.rs` is excluded by filename — it is the one
//! sanctioned definition site for `PricingLookupProvider` itself, exactly as
//! `registry.rs` is excluded from CORE-019's transport lint. `domain/pricing.rs`
//! (the file that actually defines `PricingEntity::handle_command`, the one
//! handler this lint exists to audit) is deliberately NOT excluded — the
//! provider and the handler were split into separate files precisely so
//! excluding the provider's definition site never has to exclude the
//! handler alongside it (PR3 review F-01: the original single-file version
//! excluded `pricing.rs` wholesale, which meant the scan could never see the
//! one handler it was supposed to police).

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
/// `pricing_lookup.rs` (the one sanctioned definition site for the dogfood
/// provider). Does NOT exclude `pricing.rs` — that file holds the handler
/// this lint audits.
fn collect_rs_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_rs_files(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "rs")
            && path.file_name().and_then(|n| n.to_str()) != Some("pricing_lookup.rs")
        {
            out.push(path);
        }
    }
}

struct Violation {
    location: String,
    line_text: String,
}

/// The actual detection rule, factored out from file I/O so it can be
/// proven against an in-memory source string (see
/// `lint_detects_a_handler_that_inlines_the_provider_directly` below) —
/// PR3 review F-01 found the previous version of this test suite only ever
/// proved "the real files today have zero violations", never that the scan
/// would catch a real one.
fn find_violations_in_source(label: &str, content: &str) -> Vec<Violation> {
    let mut violations = Vec::new();
    for (line_no, line) in content.lines().enumerate() {
        if line.contains("PricingLookupProvider") || line.contains("impl ExternalDataProvider") {
            violations.push(Violation {
                location: format!("{label}:{}", line_no + 1),
                line_text: line.trim().to_string(),
            });
        }
    }
    violations
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
        violations.extend(find_violations_in_source(&label, &content));
    }
    violations
}

#[test]
fn no_handler_outside_pricing_lookup_rs_constructs_the_dogfood_provider_directly() {
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
    assert!(
        !files.is_empty(),
        "workspace scan found no .rs files — CWD/workspace-root resolution is broken"
    );
    assert!(
        files
            .iter()
            .any(|f| f.file_name().and_then(|n| n.to_str()) == Some("pricing.rs")),
        "domain/pricing.rs — the file defining PricingEntity::handle_command — must be part of \
         the scanned set; if it's missing, this lint cannot see the one handler it exists to audit"
    );

    let violations = find_violations(&root, &files);

    assert!(
        violations.is_empty(),
        "found a reference-app handler constructing the dogfood provider (or a second \
         ExternalDataProvider impl) outside providers/pricing_lookup.rs — every fetch path must \
         route through the registered DataProviderAccess facade instead:\n{}",
        violations
            .iter()
            .map(|v| format!("{}: {}", v.location, v.line_text))
            .collect::<Vec<_>>()
            .join("\n")
    );
}

/// Regression control: `pricing_lookup.rs` itself defines
/// `PricingLookupProvider` and its `impl ExternalDataProvider`, so it must
/// never itself be flagged even though it obviously contains both patterns.
#[test]
fn pricing_lookup_rs_is_excluded_from_the_scan() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let root = workspace_root(&manifest_dir);
    let mut files = Vec::new();
    collect_rs_files(&root.join("examples/reference-app/src"), &mut files);

    assert!(
        !files
            .iter()
            .any(|f| f.file_name().and_then(|n| n.to_str()) == Some("pricing_lookup.rs")),
        "pricing_lookup.rs must be excluded from the scan by filename"
    );
}

/// Proves the detection rule itself catches a violation, not just that
/// today's real files happen to have none of them (PR3 review F-01's core
/// complaint). Simulates the exact hypothetical the review raised: a
/// handler-shaped file that inlines the provider directly instead of going
/// through the facade.
#[test]
fn lint_detects_a_handler_that_inlines_the_provider_directly() {
    let hypothetical_handler = r#"
async fn handle_command(&self, command: &Self::Command) -> Result<Vec<Self::Event>, EntityError> {
    let provider = PricingLookupProvider;
    provider.fetch(request).await?;
    Ok(vec![])
}
"#;

    let violations = find_violations_in_source("domain/pricing.rs", hypothetical_handler);

    assert!(
        !violations.is_empty(),
        "the detection rule failed to catch a handler that directly constructs \
         PricingLookupProvider — this is exactly the case the lint exists to prevent"
    );
}

/// Complements the above: the provider's own sanctioned definition site
/// legitimately contains both trigger substrings — this proves the
/// detection rule itself doesn't distinguish content, only file-level
/// exclusion (`collect_rs_files`) does, and that exclusion is what the two
/// tests above verify.
#[test]
fn lint_detection_rule_also_flags_the_providers_own_definition_content() {
    let provider_definition = r#"
pub struct PricingLookupProvider;

#[async_trait]
impl ExternalDataProvider for PricingLookupProvider {
    async fn fetch(&self, request: DataRequest) -> Result<DataResponse, DataProviderError> {
        todo!()
    }
}
"#;

    let violations = find_violations_in_source("providers/pricing_lookup.rs", provider_definition);

    assert!(
        !violations.is_empty(),
        "expected the provider's own definition to trip the same substrings a handler would — \
         file-level exclusion, not content-awareness, is what keeps this file out of the report"
    );
}
