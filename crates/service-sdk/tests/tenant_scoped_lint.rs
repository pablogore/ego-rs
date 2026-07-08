//! CORE-008A TASK-014 — Mandatory Seed 1: automated `#[tenant_scoped]` detection.
//!
//! AD-007's opt-in classification model has a fail-open risk: a forgotten
//! `#[tenant_scoped]` marker silently leaves an operation unenforced. This
//! test is the required automated mitigation — a `cargo test --workspace`
//! participant (this project's Strict TDD test command, already the exact
//! gate `.gitlab-ci.yml`'s `test` stage runs), not an unenforced shell script.
//!
//! # Known limitation (explicit, not hidden — AD-007)
//!
//! This is an **identifier-name heuristic**, not a security audit. It only
//! catches `#[operation]` methods that reference a tenant-related identifier
//! **directly in their own body** (`tenant_hint`, `canonical_tenant`,
//! `TenantId`, or an `ExecutionContext`-style `.tenant_id(` accessor call).
//! An operation that touches tenant-scoped data through an indirect path —
//! e.g. a repository or projection call that filters by tenant internally
//! without the operation itself naming a tenant identifier — produces a
//! **false negative** this detector cannot see. This is an accepted
//! best-effort tradeoff during the migration window (AD-007); the long-term
//! fix is the secure-by-default flip already recorded as a design.md
//! follow-up, not a progressively stronger heuristic here. Passing this test
//! must never be cited as proof an operation is correctly classified.
//!
//! # Workspace-root resolution (the CWD bug this test avoids)
//!
//! `cargo test`'s working directory is the crate root (`crates/service-sdk`),
//! not the workspace root. A literal `crates/*/src/` path relative to CWD
//! would resolve to a non-existent nested directory and silently scan zero
//! files — a passing-for-the-wrong-reason failure mode. Instead this test
//! anchors on `env!("CARGO_MANIFEST_DIR")` (fixed at compile time to this
//! crate's own directory, independent of CWD) and ascends until it finds the
//! ancestor whose `Cargo.toml` declares `[workspace]`.
//!
//! Run with: cargo test -p ego-service-sdk tenant_scoped_lint

use std::path::{Path, PathBuf};

/// Identifiers that indicate an operation body reads tenant-related state.
/// Deliberately narrow (see module doc's "Known limitation") — a broader net
/// (e.g. bare `"tenant"`) would produce false *positives* against unrelated
/// code, which would break this test's own "zero violations" gate.
const TENANT_IDENTIFIERS: [&str; 4] =
    ["tenant_hint", "canonical_tenant", "TenantId", ".tenant_id("];

struct Violation {
    location: String,
}

/// Ascends from `start` until it finds the ancestor whose `Cargo.toml`
/// declares a `[workspace]` table.
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
                "tenant_scoped_lint: could not locate workspace root ascending from {}",
                start.display()
            );
        }
    }
}

/// Recursively collects every `.rs` file under `dir`.
fn collect_rs_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_rs_files(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            out.push(path);
        }
    }
}

/// Scans `source` for `#[operation]`-annotated methods that reference a
/// tenant identifier in their body without a `#[tenant_scoped]` attribute on
/// the same method. Line-based, not a full parse — a deliberately simple
/// heuristic consistent with this test's documented best-effort scope.
fn find_violations_in_source(source: &str, file_label: &str) -> Vec<Violation> {
    let lines: Vec<&str> = source.lines().collect();
    let mut violations = Vec::new();
    let mut i = 0;

    while i < lines.len() {
        if !lines[i].trim_start().starts_with("#[operation]") {
            i += 1;
            continue;
        }

        // #[tenant_scoped] may appear immediately before or after #[operation]
        // in the attribute cluster preceding the method signature.
        let mut has_tenant_scoped = false;

        let mut b = i;
        while b > 0 {
            let t = lines[b - 1].trim();
            if t.starts_with("#[") || t.starts_with("///") || t.starts_with("//") {
                if t.starts_with("#[tenant_scoped") {
                    has_tenant_scoped = true;
                }
                b -= 1;
            } else {
                break;
            }
        }

        // Scan forward for the method signature line (contains "fn ").
        let mut sig_line_idx = None;
        let mut f = i + 1;
        while f < lines.len() && f - i <= 20 {
            let t = lines[f].trim();
            if t.starts_with("#[tenant_scoped") {
                has_tenant_scoped = true;
            }
            if t.contains("fn ") {
                sig_line_idx = Some(f);
                break;
            }
            f += 1;
        }

        let Some(sig_idx) = sig_line_idx else {
            i += 1;
            continue;
        };

        let fn_name = lines[sig_idx]
            .split("fn ")
            .nth(1)
            .and_then(|s| s.split(['(', '<']).next())
            .unwrap_or("<unknown>")
            .trim()
            .to_string();

        // Capture the body between the signature's opening `{` and its
        // matching `}`. A `;` reached before any `{` means a bodyless trait
        // declaration (today's only real production shape) — nothing to scan.
        let mut depth = 0i32;
        let mut started = false;
        let mut body = String::new();
        let mut has_body = false;

        'outer: for line in &lines[sig_idx..] {
            for ch in line.chars() {
                if !started {
                    if ch == ';' {
                        break 'outer;
                    }
                    if ch == '{' {
                        started = true;
                        has_body = true;
                        depth = 1;
                    }
                    continue;
                }
                match ch {
                    '{' => depth += 1,
                    '}' => {
                        depth -= 1;
                        if depth == 0 {
                            break 'outer;
                        }
                    }
                    _ => {}
                }
                body.push(ch);
            }
            body.push('\n');
        }

        if has_body && !has_tenant_scoped && TENANT_IDENTIFIERS.iter().any(|id| body.contains(id))
        {
            violations.push(Violation {
                location: format!("{file_label}:{} fn {}", sig_idx + 1, fn_name),
            });
        }

        i = sig_idx + 1;
    }

    violations
}

#[test]
fn tenant_scoped_lint_workspace_has_zero_violations() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let root = workspace_root(&manifest_dir);
    let crates_dir = root.join("crates");

    let mut violations = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&crates_dir) {
        for entry in entries.flatten() {
            let src_dir = entry.path().join("src");
            if !src_dir.is_dir() {
                continue;
            }
            let mut files = Vec::new();
            collect_rs_files(&src_dir, &mut files);
            for file in files {
                let Ok(content) = std::fs::read_to_string(&file) else {
                    continue;
                };
                let label = file
                    .strip_prefix(&root)
                    .unwrap_or(&file)
                    .display()
                    .to_string();
                violations.extend(find_violations_in_source(&content, &label));
            }
        }
    }

    assert!(
        !violations.is_empty() || crates_dir.is_dir(),
        "workspace scan found no crates under {} — the scan is silently \
         scanning zero files (CWD/workspace-root resolution is broken)",
        crates_dir.display()
    );

    assert!(
        violations.is_empty(),
        "found #[operation] method(s) referencing a tenant identifier without \
         #[tenant_scoped] (AD-007 fail-open mitigation):\n{}",
        violations
            .iter()
            .map(|v| v.location.as_str())
            .collect::<Vec<_>>()
            .join("\n")
    );
}

/// Proves the detector is not inert — i.e. it doesn't unconditionally return
/// an empty violation list regardless of what it scans (the "fails when
/// pointed at a deliberately-unmarked tenant-touching fixture" requirement).
/// The fixture is an in-memory literal, not a file under `crates/*/src/`, so
/// it can never itself become a real workspace violation.
#[test]
fn tenant_scoped_lint_detects_deliberately_unmarked_fixture() {
    let fixture = r#"
        #[operation]
        async fn leaks_tenant(&self, ctx: ServiceContext) -> Result<(), Err> {
            let hint = ctx.tenant_hint();
            Ok(())
        }
    "#;

    let violations = find_violations_in_source(fixture, "fixture");

    assert!(
        !violations.is_empty(),
        "detector must flag an #[operation] method referencing tenant_hint() \
         without #[tenant_scoped] — an empty result here would mean the \
         detector silently does nothing"
    );
}

/// Control case: the same tenant-touching body, correctly marked, must NOT
/// be flagged — proves the detector isn't so broad it would block Phase 6's
/// marker adoption.
#[test]
fn tenant_scoped_lint_allows_marked_operation() {
    let fixture = r#"
        #[operation]
        #[tenant_scoped]
        async fn reads_tenant(&self, ctx: ServiceContext) -> Result<(), Err> {
            let hint = ctx.tenant_hint();
            Ok(())
        }
    "#;

    let violations = find_violations_in_source(fixture, "fixture");

    assert!(
        violations.is_empty(),
        "a #[tenant_scoped]-marked operation must never be flagged"
    );
}

/// Control case: an unmarked operation whose body does NOT reference any
/// tenant identifier must NOT be flagged — proves the heuristic doesn't
/// over-fire on ordinary, genuinely tenant-less operations.
#[test]
fn tenant_scoped_lint_allows_unmarked_non_tenant_operation() {
    let fixture = r#"
        #[operation]
        async fn health_check(&self, ctx: ServiceContext) -> Result<(), Err> {
            Ok(())
        }
    "#;

    let violations = find_violations_in_source(fixture, "fixture");

    assert!(
        violations.is_empty(),
        "an unmarked operation with no tenant-identifier reference must not be flagged"
    );
}

/// A bodyless trait method declaration (today's only real production shape
/// for `#[operation]`) must never be scanned as if it had a body — proves
/// the `;`-before-`{` bodyless-declaration guard works.
#[test]
fn tenant_scoped_lint_ignores_bodyless_trait_declaration() {
    let fixture = r#"
        #[operation]
        async fn scoped_op(&self, ctx: ServiceContext) -> Result<String, Err>;
    "#;

    let violations = find_violations_in_source(fixture, "fixture");

    assert!(
        violations.is_empty(),
        "a bodyless trait method declaration must never be scanned as a violation"
    );
}

// -- CORE-008A Phase 6 (TASK-029) — FR-007 structural transport-independence --

/// Identifiers that would indicate a transport dependency leaking into the
/// runtime's tenant-resolution/enforcement path. Deliberately narrow, same
/// best-effort spirit as `TENANT_IDENTIFIERS` above — this is a structural
/// smoke check, not a full dependency-graph audit.
const TRANSPORT_IDENTIFIERS: [&str; 6] =
    ["axum", "tonic", "hyper", "HeaderMap", "grpc", "http::header"];

/// FR-007: the runtime's tenant-resolution seam (`runtime/tenant.rs`) and the
/// enforcement path it feeds (`runtime_builder.rs`) MUST reference no
/// transport-specific type or header/metadata extraction logic — only an
/// already-resolved tenant value. Extends TASK-014's scan test rather than
/// adding a third scanning mechanism (ladder rung 2: reuse what's already
/// here).
#[test]
fn runtime_tenant_enforcement_path_has_no_transport_dependency() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let root = workspace_root(&manifest_dir);

    let files = [
        root.join("crates/service-sdk/src/runtime/tenant.rs"),
        root.join("crates/service-sdk/src/runtime/runtime_builder.rs"),
    ];

    let mut found_any_file = false;
    for file in &files {
        let Ok(content) = std::fs::read_to_string(file) else {
            continue;
        };
        found_any_file = true;
        for ident in TRANSPORT_IDENTIFIERS {
            assert!(
                !content.contains(ident),
                "found transport-specific identifier {ident:?} in {} — the runtime layer \
                 must carry no transport dependency (FR-007)",
                file.display()
            );
        }
    }

    assert!(
        found_any_file,
        "neither runtime/tenant.rs nor runtime_builder.rs was found under {} — \
         the scan is silently scanning zero files",
        root.display()
    );
}
