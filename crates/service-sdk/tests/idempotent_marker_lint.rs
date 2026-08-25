//! PROD-012 follow-up — structural detection of the "mutating without
//! `#[idempotent]`" bypass.
//!
//! # The gap this closes
//!
//! `#[operation]` in `crates/service-sdk-macros/src/lib.rs` stamps every
//! generated `OperationDescriptor` with `mutating: true` unconditionally (the
//! SDK has no separate "this operation only reads" attribute today, so every
//! declared operation IS the mutating case). `#[idempotent]` is a fully
//! optional, separate attribute. Nothing previously read the `mutating` flag
//! to gate, lint, or refuse a mutating operation missing `#[idempotent]` at
//! build time — a service built on this SDK could ship a mutating command
//! with no replay/conflict/durable-receipt protection and it would compile
//! and dispatch fine. `examples/reference-app/tests/idempotent_marker_completeness.rs`
//! (#290) checks this, but only for reference-app's own hand-maintained
//! inventory — it says nothing about any other crate built on the SDK.
//!
//! This mirrors `tenant_scoped_lint.rs`'s mechanism exactly: a `syn`-based
//! AST scan over every workspace member's `src/`, run as a `cargo test`
//! participant (this project's Strict TDD gate), rather than a
//! pipeline-specific script. See that file for the fuller rationale on why a
//! `cargo test` is the enforcement point rather than a macro-time
//! `compile_error!` — the short version: it lets one scan cover every crate
//! built against the SDK, not just the one being compiled.
//!
//! # Scope: `src/`, not `tests/`, and not `#[cfg(test)]` modules
//!
//! Like `tenant_scoped_lint.rs`, this only scans `src/` under each workspace
//! member (`crates/*` and `examples/*` — the latter added here because
//! `examples/reference-app` is the one real host on this SDK today and the
//! whole point is to catch this for hosts, not just the SDK's own crates).
//! `tests/` fixtures need the freedom to declare operations for unrelated
//! purposes (testing `#[authorize]`, tenant scoping, codegen shape, etc.)
//! without being dragged into idempotency bookkeeping, so they are never
//! visited at all.
//!
//! Within `src/`, this additionally skips any `#[cfg(test)] mod { .. }`
//! block — a refinement `tenant_scoped_lint.rs` does not need (it currently
//! has no counter-example), but this rule does: `crates/transport/src/state.rs`
//! and `crates/testkit/src/fixtures.rs` both declare `#[operation]` methods
//! inside inline `#[cfg(test)]` modules purely as fixtures for unrelated
//! tests. They never ship, so flagging them would be a false positive against
//! legitimate code, not a caught bypass.
//!
//! Run with: cargo test -p ego-service-sdk idempotent_marker_lint

use std::path::{Path, PathBuf};

use syn::{Item, ItemTrait, TraitItem};

struct Violation {
    location: String,
}

/// Ascends from `start` until it finds the ancestor whose `Cargo.toml`
/// declares a `[workspace]` table. Duplicated from `tenant_scoped_lint.rs`
/// per this codebase's established per-lint-file convention (each `tests/`
/// integration binary compiles standalone and cannot share a module).
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
                "idempotent_marker_lint: could not locate workspace root ascending from {}",
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

fn has_attr(attrs: &[syn::Attribute], name: &str) -> bool {
    attrs.iter().any(|a| a.path().is_ident(name))
}

/// Whether `attrs` contains `#[cfg(test)]` specifically (not just any `cfg`).
fn is_cfg_test(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|a| {
        a.path().is_ident("cfg")
            && a.parse_args::<syn::Ident>()
                .is_ok_and(|ident| ident == "test")
    })
}

/// Walks `items`, recursing into inline `mod { .. }` bodies but skipping any
/// module attributed `#[cfg(test)]` (see module doc), invoking `on_trait` for
/// every trait item found.
fn walk_items<'a>(items: &'a [Item], on_trait: &mut impl FnMut(&'a ItemTrait)) {
    for item in items {
        match item {
            Item::Trait(t) => on_trait(t),
            Item::Mod(m) => {
                if is_cfg_test(&m.attrs) {
                    continue;
                }
                if let Some((_, inner)) = &m.content {
                    walk_items(inner, on_trait);
                }
            }
            _ => {}
        }
    }
}

/// Every `#[operation]` trait method not also carrying `#[idempotent]` is a
/// violation: the macro stamps `mutating: true` on it unconditionally, and
/// nothing else in the workspace protects a retry of it.
fn find_violations(files: &[(String, syn::File)]) -> Vec<Violation> {
    let mut violations = Vec::new();

    for (label, file) in files {
        let mut on_trait = |t: &ItemTrait| {
            for item in &t.items {
                if let TraitItem::Fn(m) = item {
                    if has_attr(&m.attrs, "operation") && !has_attr(&m.attrs, "idempotent") {
                        violations.push(Violation {
                            location: format!("{label}: trait {} fn {}", t.ident, m.sig.ident),
                        });
                    }
                }
            }
        };
        walk_items(&file.items, &mut on_trait);
    }

    violations
}

/// Test convenience: parses a single source string and scans it.
fn find_violations_in_source(source: &str, label: &str) -> Vec<Violation> {
    let file = match syn::parse_file(source) {
        Ok(f) => f,
        Err(e) => panic!("idempotent_marker_lint fixture failed to parse: {e}"),
    };
    find_violations(&[(label.to_string(), file)])
}

#[test]
fn idempotent_marker_lint_workspace_has_zero_violations() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let root = workspace_root(&manifest_dir);

    let member_src_dirs = ["crates", "examples"];

    let mut files = Vec::new();
    for members_dir in member_src_dirs {
        let Ok(entries) = std::fs::read_dir(root.join(members_dir)) else {
            continue;
        };
        for entry in entries.flatten() {
            let src_dir = entry.path().join("src");
            if !src_dir.is_dir() {
                continue;
            }
            let mut rs_files = Vec::new();
            collect_rs_files(&src_dir, &mut rs_files);
            for file in rs_files {
                let Ok(content) = std::fs::read_to_string(&file) else {
                    continue;
                };
                let label = file
                    .strip_prefix(&root)
                    .unwrap_or(&file)
                    .display()
                    .to_string();
                let Ok(parsed) = syn::parse_file(&content) else {
                    continue; // not this test's job to report unrelated parse errors
                };
                files.push((label, parsed));
            }
        }
    }

    assert!(
        !files.is_empty(),
        "workspace scan found no member src/ files under {} — the scan is silently \
         scanning zero files (CWD/workspace-root resolution is broken)",
        root.display()
    );

    let violations = find_violations(&files);

    assert!(
        violations.is_empty(),
        "found #[operation] method(s) without #[idempotent] (PROD-012 fail-open \
         bypass — every operation is generated with mutating: true and nothing else \
         protects a retry of it):\n{}",
        violations
            .iter()
            .map(|v| v.location.as_str())
            .collect::<Vec<_>>()
            .join("\n")
    );
}

/// The defect this file exists to catch: a mutating `#[operation]` with no
/// `#[idempotent]` compiles fine today. Must be flagged.
#[test]
fn idempotent_marker_lint_detects_mutating_operation_without_idempotent() {
    let fixture = r#"
        #[service(version = "1.0.0")]
        trait PaymentService {
            #[operation]
            async fn charge(&self, ctx: ServiceContext, cmd: Charge) -> Result<Receipt, Err>;
        }
    "#;

    let violations = find_violations_in_source(fixture, "fixture");

    assert!(
        !violations.is_empty(),
        "detector must flag an #[operation] method lacking #[idempotent] — the \
         exact structural bypass PROD-012 was supposed to close"
    );
}

/// Control case: the same operation, correctly marked. Must NOT be flagged.
#[test]
fn idempotent_marker_lint_allows_marked_operation() {
    let fixture = r#"
        #[service(version = "1.0.0")]
        trait PaymentService {
            #[operation]
            #[idempotent]
            async fn charge(&self, ctx: ServiceContext, cmd: Charge) -> Result<Receipt, Err>;
        }
    "#;

    let violations = find_violations_in_source(fixture, "fixture");

    assert!(
        violations.is_empty(),
        "an #[idempotent]-marked operation must never be flagged"
    );
}

/// Control case: a plain trait method with no `#[operation]` at all must not
/// be flagged — proves the lint doesn't over-fire on ordinary trait methods,
/// and in particular never demands `#[idempotent]` on a non-operation (i.e.
/// non-mutating, per this SDK's only classification today) method.
#[test]
fn idempotent_marker_lint_ignores_non_operation_methods() {
    let fixture = r#"
        trait HealthService {
            async fn health_check(&self, ctx: ServiceContext) -> Result<(), Err>;
        }
    "#;

    let violations = find_violations_in_source(fixture, "fixture");

    assert!(
        violations.is_empty(),
        "a method without #[operation] must never be flagged"
    );
}

/// Control case: an `#[operation]` method declared inside a `#[cfg(test)]`
/// module must not be flagged — it is test-fixture scaffolding, not a
/// shipped operation, and this is the exact shape of the two real
/// counter-examples this lint's scope was designed against (see module doc).
#[test]
fn idempotent_marker_lint_ignores_cfg_test_modules() {
    let fixture = r#"
        #[cfg(test)]
        mod tests {
            #[service(version = "1.0.0")]
            pub trait Echo {
                #[operation]
                async fn echo(&self, ctx: ServiceContext, input: String) -> Result<String, Err>;
            }
        }
    "#;

    let violations = find_violations_in_source(fixture, "fixture");

    assert!(
        violations.is_empty(),
        "an #[operation] method inside a #[cfg(test)] module must never be flagged"
    );
}
