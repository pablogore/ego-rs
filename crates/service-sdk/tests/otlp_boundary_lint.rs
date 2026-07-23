//! OTLP boundary lint (PROD-003 Phase 7).
//!
//! The ambient OTel/`tracing` context lookups `Context::current()` /
//! `Span::current()` MUST NOT appear in production code OUTSIDE the single
//! infrastructure OTLP adapter (`crates/infrastructure/src/tracing_otlp.rs`),
//! which is the only place allowed to touch the vendor SDK's ambient API as an
//! internal detail. Anywhere else they would re-introduce the ambient
//! trace-context propagation PROD-003 (and EGO's architecture) deliberately
//! forbids — trace identity travels explicitly on `ServiceContext`.
//!
//! This is a source-scan safety net (mirrors `tenant_scoped_lint.rs`): it turns
//! the "no ambient state" invariant from a documented rule into a CI-enforced
//! one. Doc-comment mentions of the forbidden symbols are ignored (only code is
//! scanned), so the invariant can still be described in prose.
//!
//! `cargo test`'s CWD is the crate root, not the workspace root, so paths anchor
//! on `env!("CARGO_MANIFEST_DIR")` and walk up to the workspace `Cargo.toml`.

use std::path::{Path, PathBuf};

/// Ambient-context lookups that must not appear in production code outside the
/// adapter.
const FORBIDDEN: [&str; 2] = ["Context::current", "Span::current"];

/// The one production file allowed to reference the ambient OTel API (its use
/// there — if any — is an adapter-internal detail, never framework-context
/// propagation).
const ADAPTER_REL: &str = "crates/infrastructure/src/tracing_otlp.rs";

/// Walk up from `start` to the directory whose `Cargo.toml` declares
/// `[workspace]`.
fn workspace_root(start: &Path) -> PathBuf {
    let mut dir = start;
    loop {
        let candidate = dir.join("Cargo.toml");
        if let Ok(content) = std::fs::read_to_string(&candidate) {
            if content.contains("[workspace]") {
                return dir.to_path_buf();
            }
        }
        dir = dir
            .parent()
            .expect("reached filesystem root without a [workspace] Cargo.toml");
    }
}

/// Drop `//`-to-end-of-line comments so doc/line-comment prose that merely
/// *mentions* a forbidden symbol is not flagged — only code is scanned.
fn strip_line_comments(src: &str) -> String {
    src.lines()
        .map(|line| line.split("//").next().unwrap_or(""))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The first forbidden pattern present in `code` (comments already stripped), if any.
fn first_forbidden(code: &str) -> Option<&'static str> {
    FORBIDDEN.into_iter().find(|p| code.contains(p))
}

/// Recursively collect `.rs` files under `dir` (skipping `target/`).
fn collect_rs(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if path.file_name().is_some_and(|n| n == "target") {
                continue;
            }
            collect_rs(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

// -- Detector unit tests (RED fixtures) --

#[test]
fn detector_flags_ambient_context_lookup_in_code() {
    let src = "fn handler() { let cx = opentelemetry::Context::current(); let _ = cx; }";
    assert_eq!(
        first_forbidden(&strip_line_comments(src)),
        Some("Context::current")
    );
}

#[test]
fn detector_flags_ambient_span_lookup_in_code() {
    let src = "fn handler() { let s = tracing::Span::current(); let _ = s; }";
    assert_eq!(
        first_forbidden(&strip_line_comments(src)),
        Some("Span::current")
    );
}

#[test]
fn detector_ignores_a_forbidden_symbol_that_only_appears_in_a_comment() {
    let src = "// `Context::current()` / `Span::current()` are forbidden here.\nfn handler() {}";
    assert_eq!(first_forbidden(&strip_line_comments(src)), None);
}

// -- Workspace scan --

#[test]
fn otlp_boundary_lint_workspace_has_zero_violations() {
    let root = workspace_root(&PathBuf::from(env!("CARGO_MANIFEST_DIR")));
    let adapter = root.join(ADAPTER_REL);

    // Production source only: `crates/*/src` and `examples/*/src`.
    let mut files = Vec::new();
    for base in [root.join("crates"), root.join("examples")] {
        let Ok(members) = std::fs::read_dir(&base) else {
            continue;
        };
        for member in members.flatten() {
            collect_rs(&member.path().join("src"), &mut files);
        }
    }
    assert!(
        !files.is_empty(),
        "found no source files to scan — path anchoring is wrong"
    );

    let mut violations = Vec::new();
    for file in files {
        if file == adapter {
            continue; // the adapter is the one allowed site
        }
        let Ok(src) = std::fs::read_to_string(&file) else {
            continue;
        };
        if let Some(pat) = first_forbidden(&strip_line_comments(&src)) {
            violations.push(format!("{}: uses `{pat}`", file.display()));
        }
    }

    assert!(
        violations.is_empty(),
        "ambient OTel/tracing context lookups found in production code outside \
         `{ADAPTER_REL}` — trace identity must travel explicitly on `ServiceContext`, \
         never via ambient state:\n{}",
        violations.join("\n")
    );
}
