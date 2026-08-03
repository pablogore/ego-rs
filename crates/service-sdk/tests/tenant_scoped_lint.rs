//! CORE-008A TASK-014 — Mandatory Seed 1: automated `#[tenant_scoped]` detection.
//!
//! AD-007's opt-in classification model has a fail-open risk: a forgotten
//! `#[tenant_scoped]` marker silently leaves an operation unenforced. This
//! test is the required automated mitigation — a `cargo test --workspace`
//! participant, which is this project's Strict TDD test command and part of the
//! declared gate set, rather than an unenforced shell script. It therefore runs
//! wherever the suite runs, with no pipeline-specific wiring of its own.
//!
//! # AST-based, not line-based (code-review fix)
//!
//! The original version of this test matched `#[operation]` immediately
//! preceding a method and scanned that SAME item's body for tenant
//! identifiers. That structurally could never catch anything real: every
//! `#[operation]` in this codebase's convention is declared on a bodyless
//! trait method (`;`-terminated) — see `crates/service-sdk/examples/order_service.rs`
//! — while the actual logic lives in a separate, unattributed
//! `impl Trait for Struct` block the old scanner never visited. The
//! "zero violations" result was vacuous, not a guarantee.
//!
//! This version parses each file with `syn` and does two passes: (1) collect,
//! per trait, which `#[operation]` methods are also `#[tenant_scoped]`; (2)
//! for every `impl Trait for X` block, check each method's REAL body — the
//! one that actually runs — against pass 1's classification for that trait.
//!
//! # Known limitation (explicit, not hidden — AD-007)
//!
//! This is still an **identifier-name heuristic**, not a security audit. It
//! flags an impl method only when its body references a tenant-related
//! identifier directly (`tenant_hint`, `canonical_tenant`, `TenantId`,
//! `tenant_id`) AND its trait's `#[operation]` declaration lacks
//! `#[tenant_scoped]`. An operation that touches tenant-scoped data through
//! an indirect path (e.g. a repository call that filters by tenant
//! internally, several calls removed from any of these identifiers) produces
//! a **false negative** this detector cannot see. This is an accepted
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

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use syn::visit::Visit;
use syn::{Item, ItemImpl, ItemTrait, TraitItem};

/// Identifiers that indicate a method body reads tenant-related state.
/// Deliberately narrow (see module doc's "Known limitation") — a broader net
/// (e.g. bare `"tenant"`) would produce false *positives* against unrelated
/// code, which would break this test's own "zero violations" gate.
const TENANT_IDENTIFIERS: [&str; 4] = ["tenant_hint", "canonical_tenant", "TenantId", "tenant_id"];

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

fn has_attr(attrs: &[syn::Attribute], name: &str) -> bool {
    attrs.iter().any(|a| a.path().is_ident(name))
}

/// Walks `items`, recursing into inline `mod { ... }` bodies (external
/// `mod foo;` declarations have no inline content and are not followed —
/// this scan operates per already-collected file, not across the module
/// tree), invoking `on_trait`/`on_impl` for every trait/impl item found.
fn walk_items<'a>(
    items: &'a [Item],
    on_trait: &mut impl FnMut(&'a ItemTrait),
    on_impl: &mut impl FnMut(&'a ItemImpl),
) {
    for item in items {
        match item {
            Item::Trait(t) => on_trait(t),
            Item::Impl(i) => on_impl(i),
            Item::Mod(m) => {
                if let Some((_, inner)) = &m.content {
                    walk_items(inner, on_trait, on_impl);
                }
            }
            _ => {}
        }
    }
}

/// Collects every identifier name appearing anywhere in a syntax subtree.
/// Used instead of a raw string `contains` check on the body so formatting
/// (line breaks, spacing around `.method(`) can never hide or fake a match.
#[derive(Default)]
struct IdentCollector {
    found: HashSet<String>,
}

impl<'ast> Visit<'ast> for IdentCollector {
    fn visit_ident(&mut self, ident: &'ast proc_macro2::Ident) {
        self.found.insert(ident.to_string());
    }
}

fn body_references_tenant_identifier(block: &syn::Block) -> bool {
    let mut collector = IdentCollector::default();
    collector.visit_block(block);
    TENANT_IDENTIFIERS
        .iter()
        .any(|id| collector.found.contains(*id))
}

/// Pass 1: `trait_name -> method_name -> has_tenant_scoped`, for every method
/// carrying `#[operation]` (the only methods the runtime treats as tenant
/// classification targets at all).
fn collect_trait_operations(
    files: &[(String, syn::File)],
) -> HashMap<String, HashMap<String, bool>> {
    let mut trait_ops: HashMap<String, HashMap<String, bool>> = HashMap::new();
    for (_, file) in files {
        let mut on_trait = |t: &ItemTrait| {
            let entry = trait_ops.entry(t.ident.to_string()).or_default();
            for item in &t.items {
                if let TraitItem::Fn(m) = item {
                    if has_attr(&m.attrs, "operation") {
                        entry.insert(m.sig.ident.to_string(), has_attr(&m.attrs, "tenant_scoped"));
                    }
                }
            }
        };
        let mut on_impl = |_: &ItemImpl| {};
        walk_items(&file.items, &mut on_trait, &mut on_impl);
    }
    trait_ops
}

/// Pass 2: for every `impl Trait for X` method whose trait marks it
/// `#[operation]` but NOT `#[tenant_scoped]` (per pass 1), check the method's
/// real body — the one that actually executes — for a tenant identifier.
fn find_violations(files: &[(String, syn::File)]) -> Vec<Violation> {
    let trait_ops = collect_trait_operations(files);
    let mut violations = Vec::new();

    for (label, file) in files {
        let mut on_trait = |_: &ItemTrait| {};
        let mut on_impl = |i: &ItemImpl| {
            let Some((_, trait_path, _)) = &i.trait_ else {
                return; // inherent impl, not a trait impl — nothing to classify
            };
            let Some(trait_name) = trait_path.segments.last().map(|s| s.ident.to_string()) else {
                return;
            };
            let Some(methods) = trait_ops.get(&trait_name) else {
                return; // not a trait this scan has any #[operation] record for
            };
            for item in &i.items {
                if let syn::ImplItem::Fn(m) = item {
                    let method_name = m.sig.ident.to_string();
                    let Some(&is_tenant_scoped) = methods.get(&method_name) else {
                        continue; // not an #[operation] method on this trait
                    };
                    if !is_tenant_scoped && body_references_tenant_identifier(&m.block) {
                        violations.push(Violation {
                            location: format!("{label}: impl {trait_name} fn {method_name}"),
                        });
                    }
                }
            }
        };
        walk_items(&file.items, &mut on_trait, &mut on_impl);
    }

    violations
}

/// Test convenience: parses a single source string (trait + its impl,
/// together, as real fixtures in this codebase always are) and scans it.
fn find_violations_in_source(source: &str, label: &str) -> Vec<Violation> {
    let file = match syn::parse_file(source) {
        Ok(f) => f,
        Err(e) => panic!("tenant_scoped_lint fixture failed to parse: {e}"),
    };
    find_violations(&[(label.to_string(), file)])
}

#[test]
fn tenant_scoped_lint_workspace_has_zero_violations() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let root = workspace_root(&manifest_dir);
    let crates_dir = root.join("crates");

    let mut files = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&crates_dir) {
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
        "workspace scan found no crates under {} — the scan is silently \
         scanning zero files (CWD/workspace-root resolution is broken)",
        crates_dir.display()
    );

    let violations = find_violations(&files);

    assert!(
        violations.is_empty(),
        "found #[operation] impl method(s) referencing a tenant identifier without \
         #[tenant_scoped] on the trait declaration (AD-007 fail-open mitigation):\n{}",
        violations
            .iter()
            .map(|v| v.location.as_str())
            .collect::<Vec<_>>()
            .join("\n")
    );
}

/// The regression test for the original defect: a bodyless trait declaration
/// (today's only real production shape for `#[operation]`) paired with a
/// SEPARATE `impl Trait for Struct` block carrying the real, tenant-touching
/// body. The old line-based scanner only ever looked at the trait
/// declaration and could never see this. This must be flagged.
#[test]
fn tenant_scoped_lint_detects_violation_in_impl_block_not_trait_declaration() {
    let fixture = r#"
        trait LeakyService {
            #[operation]
            async fn leaks_tenant(&self, ctx: ServiceContext) -> Result<(), Err>;
        }

        struct LeakyServiceImpl;

        impl LeakyService for LeakyServiceImpl {
            async fn leaks_tenant(&self, ctx: ServiceContext) -> Result<(), Err> {
                let hint = ctx.tenant_hint();
                Ok(())
            }
        }
    "#;

    let violations = find_violations_in_source(fixture, "fixture");

    assert!(
        !violations.is_empty(),
        "detector must flag an #[operation] impl method referencing tenant_hint() \
         in its real body when the trait declaration lacks #[tenant_scoped] — the \
         exact shape every real #[operation] takes in this codebase"
    );
}

/// Control case: the same tenant-touching impl body, but the trait
/// declaration correctly marks the method `#[tenant_scoped]` — must NOT be
/// flagged, proving the detector isn't so broad it would block legitimate
/// marker adoption.
#[test]
fn tenant_scoped_lint_allows_marked_operation() {
    let fixture = r#"
        trait ScopedService {
            #[operation]
            #[tenant_scoped]
            async fn reads_tenant(&self, ctx: ServiceContext) -> Result<(), Err>;
        }

        struct ScopedServiceImpl;

        impl ScopedService for ScopedServiceImpl {
            async fn reads_tenant(&self, ctx: ServiceContext) -> Result<(), Err> {
                let hint = ctx.tenant_hint();
                Ok(())
            }
        }
    "#;

    let violations = find_violations_in_source(fixture, "fixture");

    assert!(
        violations.is_empty(),
        "a #[tenant_scoped]-marked operation must never be flagged, even though its \
         impl body references a tenant identifier"
    );
}

/// Control case: an unmarked operation whose impl body does NOT reference
/// any tenant identifier must NOT be flagged — proves the heuristic doesn't
/// over-fire on ordinary, genuinely tenant-less operations.
#[test]
fn tenant_scoped_lint_allows_unmarked_non_tenant_operation() {
    let fixture = r#"
        trait HealthService {
            #[operation]
            async fn health_check(&self, ctx: ServiceContext) -> Result<(), Err>;
        }

        struct HealthServiceImpl;

        impl HealthService for HealthServiceImpl {
            async fn health_check(&self, ctx: ServiceContext) -> Result<(), Err> {
                Ok(())
            }
        }
    "#;

    let violations = find_violations_in_source(fixture, "fixture");

    assert!(
        violations.is_empty(),
        "an unmarked operation whose impl body has no tenant-identifier reference \
         must not be flagged"
    );
}

/// The trait declaration itself is bodyless (`;`-terminated) and is never
/// scanned for tenant identifiers — only its `#[tenant_scoped]` presence is
/// read from it. Even if the trait's surrounding text mentions a tenant
/// identifier (e.g. in a doc comment, or a return type named `TenantId`),
/// that must never produce a violation on its own.
#[test]
fn tenant_scoped_lint_ignores_trait_declaration_text_itself() {
    let fixture = r#"
        trait LookupService {
            /// Returns the canonical_tenant for diagnostics.
            #[operation]
            async fn lookup(&self, ctx: ServiceContext) -> Result<TenantId, Err>;
        }

        struct LookupServiceImpl;

        impl LookupService for LookupServiceImpl {
            async fn lookup(&self, ctx: ServiceContext) -> Result<TenantId, Err> {
                Ok(default_tenant_id())
            }
        }
    "#;

    let violations = find_violations_in_source(fixture, "fixture");

    assert!(
        violations.is_empty(),
        "the trait declaration's doc comment and return type must never themselves \
         trigger a violation — only the impl body's own identifiers count, and this \
         impl body references no TENANT_IDENTIFIERS token"
    );
}

/// A method appearing only as an inherent impl (no trait), or on a trait this
/// scan has no `#[operation]` record for, must be skipped rather than
/// guessed at — avoids false positives on unrelated code.
#[test]
fn tenant_scoped_lint_ignores_inherent_impl_and_untracked_traits() {
    let fixture = r#"
        struct Standalone;

        impl Standalone {
            async fn touches_tenant_hint(&self, ctx: ServiceContext) -> Result<(), Err> {
                let hint = ctx.tenant_hint();
                Ok(())
            }
        }

        trait UnrelatedTrait {
            async fn plain_method(&self, ctx: ServiceContext) -> Result<(), Err>;
        }

        impl UnrelatedTrait for Standalone {
            async fn plain_method(&self, ctx: ServiceContext) -> Result<(), Err> {
                let hint = ctx.tenant_hint();
                Ok(())
            }
        }
    "#;

    let violations = find_violations_in_source(fixture, "fixture");

    assert!(
        violations.is_empty(),
        "an inherent impl method, and a method on a trait with no #[operation] \
         methods recorded, must not be flagged — this scan only classifies methods \
         the framework itself treats as operations"
    );
}

/// Regression for the AD-011 resolved-tenant READ path specifically. The
/// existing detection fixture above exercises `tenant_hint()` — the
/// caller-supplied INPUT accessor. The more security-relevant "tenant-touching"
/// signal is an operation that READS the resolved tenant via the AD-011 read
/// accessor `ServiceContext::canonical_tenant()` (context/mod.rs) yet forgot
/// `#[tenant_scoped]` on its trait declaration: it consumes tenant-scoped state
/// while never calling `enforce_tenant` (AD-007 fail-open). This must be flagged.
#[test]
fn tenant_scoped_lint_detects_violation_reading_resolved_tenant() {
    let fixture = r#"
        trait ReportService {
            #[operation]
            async fn export(&self, ctx: ServiceContext) -> Result<(), Err>;
        }

        struct ReportServiceImpl;

        impl ReportService for ReportServiceImpl {
            async fn export(&self, ctx: ServiceContext) -> Result<(), Err> {
                let tenant = ctx.canonical_tenant();
                Ok(())
            }
        }
    "#;

    let violations = find_violations_in_source(fixture, "fixture");

    assert!(
        !violations.is_empty(),
        "detector must flag an #[operation] impl method that reads the resolved \
         tenant via canonical_tenant() (the AD-011 read accessor) when the trait \
         declaration lacks #[tenant_scoped] — this is a tenant-touching operation \
         that never calls enforce_tenant (AD-007 fail-open)"
    );
}

/// Control for the case above: the same resolved-tenant-reading impl body, but
/// the trait declaration correctly carries `#[tenant_scoped]` — must NOT be
/// flagged, proving the guard does not block legitimate marker adoption on the
/// AD-011 read path.
#[test]
fn tenant_scoped_lint_allows_marked_operation_reading_resolved_tenant() {
    let fixture = r#"
        trait ReportService {
            #[operation]
            #[tenant_scoped]
            async fn export(&self, ctx: ServiceContext) -> Result<(), Err>;
        }

        struct ReportServiceImpl;

        impl ReportService for ReportServiceImpl {
            async fn export(&self, ctx: ServiceContext) -> Result<(), Err> {
                let tenant = ctx.canonical_tenant();
                Ok(())
            }
        }
    "#;

    let violations = find_violations_in_source(fixture, "fixture");

    assert!(
        violations.is_empty(),
        "a #[tenant_scoped]-marked operation reading canonical_tenant() must never \
         be flagged, even though its impl body references the resolved-tenant accessor"
    );
}

// -- CORE-008A Phase 6 (TASK-029) — FR-007 structural transport-independence --

/// Identifiers that would indicate a transport dependency leaking into the
/// runtime's tenant-resolution/enforcement path. Deliberately narrow, same
/// best-effort spirit as `TENANT_IDENTIFIERS` above — this is a structural
/// smoke check, not a full dependency-graph audit.
const TRANSPORT_IDENTIFIERS: [&str; 6] = [
    "axum",
    "tonic",
    "hyper",
    "HeaderMap",
    "grpc",
    "http::header",
];

/// FR-007: the runtime's tenant-resolution seam (`runtime/tenant.rs`) and the
/// enforcement path it feeds (`runtime_builder.rs`) MUST reference no
/// transport-specific type or header/metadata extraction logic — only an
/// already-resolved tenant value. Extends TASK-014's scan test rather than
/// adding a third scanning mechanism (ladder rung 2: reuse what's already
/// here). This check is a plain text scan, unrelated to the AST-based
/// #[tenant_scoped] classification above.
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
