//! The ledger guard: `README.md`, the module registration and the directory must
//! describe the same set of tests.
//!
//! # Why this exists
//!
//! `README.md` carries a ledger — a budget, a category per test, and a
//! justification for each. It is the only place that records *why* a test was
//! admitted, and the admission rules say a scenario with no justification is not
//! admitted at all.
//!
//! That ledger had already drifted. It declared six infrastructure tests while ten
//! existed, so four had been added with no recorded justification and no budget
//! entry — and nothing failed, because nothing compared the two. A ledger that
//! nobody checks stops describing the suite and starts describing whatever it
//! described last.
//!
//! # What it compares, and why three sources rather than two
//!
//! Three descriptions of the same set, each of which can drift independently:
//!
//! | Source | Drifts when |
//! |---|---|
//! | `tests/infrastructure/*.rs` | a file is added, deleted or renamed |
//! | `tests/infrastructure.rs` | a file exists but is never registered as a module, so it compiles nowhere and runs never |
//! | `README.md` | a test is added or renamed without recording what it guarantees or which budget it spends |
//!
//! Comparing only the directory against the README would miss the second case
//! entirely: a file present, documented, and silently not compiled. That failure is
//! invisible in the worst way — the suite reports success and the scenario never
//! ran. So all three are required to agree, and every disagreement is reported from
//! the side that has the extra or missing entry rather than as one opaque
//! inequality.
//!
//! # Deliberately hermetic
//!
//! This test starts no container, opens no connection and reads no environment. It
//! is a separate target from `infrastructure` for exactly that reason: it runs
//! without Docker, and the runner executes it **before** provisioning PostgreSQL,
//! so a drifted ledger fails in milliseconds instead of after a container start.
//!
//! ```bash
//! cargo test --manifest-path integration-tests/Cargo.toml --test ledger
//! ```
//!
//! # What it does not check
//!
//! That a justification is *true*. Nothing here reads the prose. This guard proves
//! every test has a row and every row has a test; whether the row says something
//! accurate is a review question, and `README.md` says out loud that a
//! nearly-right justification is the one that survives review.

use std::collections::BTreeSet;
use std::path::Path;

/// The directory holding one file per infrastructure test.
const TEST_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/infrastructure");

/// The target that registers each file as a module. A file missing from here
/// compiles nowhere and runs never.
const REGISTRATION: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/infrastructure.rs");

/// The ledger itself — the budget, the categories and the justifications.
const README: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/README.md");

/// Every `*.rs` file in `tests/infrastructure/`, by module name.
fn modules_on_disk() -> BTreeSet<String> {
    let dir = Path::new(TEST_DIR);
    let entries = std::fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("the infrastructure test directory must be readable: {e}"));

    let mut found = BTreeSet::new();
    for entry in entries {
        let path = entry.expect("each directory entry must be readable").path();
        if path.extension().and_then(|e| e.to_str()) != Some("rs") {
            continue;
        }
        let stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .expect("a .rs file must have a UTF-8 stem")
            .to_owned();
        found.insert(stem);
    }
    found
}

/// Every `mod <name>;` declared by the registration target.
///
/// Matched on the trailing semicolon, so the enclosing `mod infrastructure {`
/// block header is not mistaken for a test module. Attributes such as
/// `#[cfg(unix)]` sit on their own lines and do not interfere; a module gated to
/// one platform is still *registered* on every platform, which is the property
/// this ledger tracks.
fn modules_registered() -> BTreeSet<String> {
    let source = std::fs::read_to_string(REGISTRATION)
        .unwrap_or_else(|e| panic!("the registration target must be readable: {e}"));
    registered_modules(&source)
}

/// The parse behind [`modules_registered`], over text rather than a path.
///
/// Split out for the same reason as [`ledger_citations`]: a function that reads
/// a fixed file can only ever be asserted against the current tree.
fn registered_modules(source: &str) -> BTreeSet<String> {
    let mut found = BTreeSet::new();
    for line in source.lines() {
        let line = line.trim();
        let Some(rest) = line.strip_prefix("mod ") else {
            continue;
        };
        let Some(name) = rest.strip_suffix(';') else {
            continue;
        };
        let name = name.trim();
        if is_module_name(name) {
            found.insert(name.to_owned());
        }
    }
    found
}

/// Every test the ledger accounts for, counted **only** from a real table row.
///
/// A test is accounted for by the Status cell of a row that also carries its
/// guarantee, its why-in-process and the budget it spends. Two anchors are
/// required, and each closes a different hole:
///
/// 1. **The line must be a table row** — trimmed, it starts with `|`.
/// 2. **The path must be a code span** — delimited by backticks, as every Status
///    cell writes it.
///
/// # Why anchoring is not a refinement
///
/// An earlier version scanned the whole document for the bare path, and that was
/// a live false green rather than a theoretical one. This README is deliberately
/// narrative and cites test paths in prose: `concurrent_replicas_postgres.rs` is
/// named both in its Status cell *and* in the paragraph recording that the
/// scenario is now guarded. Under the unanchored scan, deleting that row — the
/// only place its justification lives — left the prose mention behind, the name
/// still in this set, and the guard green while claiming every test has a row.
///
/// The guarantee this file states is that every test carries a *justification*.
/// A name appearing somewhere in the file is not that, so the parse is anchored
/// to the structure that actually holds one.
fn modules_in_ledger() -> BTreeSet<String> {
    let ledger = std::fs::read_to_string(README)
        .unwrap_or_else(|e| panic!("the ledger must be readable: {e}"));
    ledger_citations(&ledger)
}

/// The parse behind [`modules_in_ledger`], over text rather than a path.
///
/// **This split is the point, not a tidy-up.** The anchoring is the load-bearing
/// behaviour of this file, and while the parse read a fixed compile-time path it
/// could not be exercised against anything but the current tree. Weakening it —
/// dropping the table-row check, say — left every test in this target green,
/// because the real README happens to satisfy both the strict and the loose rule
/// for nine of its ten entries. The guard would have returned to the exact
/// false-green state it was written to close, silently.
///
/// Taking `&str` makes the rule assertable against fixtures, which is what the
/// tests at the bottom of this file do. A prose mutation table is a record of
/// something someone ran once; these run every time.
fn ledger_citations(markdown: &str) -> BTreeSet<String> {
    // Backticks are part of both delimiters: a Status cell writes the path as a
    // code span, and requiring that rejects a path mentioned in a row's prose.
    const OPEN: &str = "`tests/infrastructure/";
    const CLOSE: &str = ".rs`";

    let mut found = BTreeSet::new();
    for line in markdown.lines() {
        if !line.trim_start().starts_with('|') {
            continue;
        }
        let mut rest = line;
        while let Some(start) = rest.find(OPEN) {
            rest = &rest[start + OPEN.len()..];
            let Some(end) = rest.find(CLOSE) else {
                continue;
            };
            let name = &rest[..end];
            if is_module_name(name) {
                found.insert(name.to_owned());
            }
        }
    }
    found
}

/// A Rust module name as this suite writes them: lowercase, digits, underscores.
///
/// A last filter rather than the anchor. What keeps prose out of the ledger set
/// is the table-row and code-span anchoring above; this only rejects a match
/// whose captured text could not be a module name at all — a placeholder like
/// `<name>`, or a span that ran past its own delimiter into the rest of a line.
/// It fails closed: an unrecognised shape is dropped, never guessed at.
fn is_module_name(candidate: &str) -> bool {
    !candidate.is_empty()
        && candidate
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

/// Renders one side's surplus as a readable failure.
fn report(missing_from: &str, present_in: &str, names: &BTreeSet<String>) -> String {
    let list = names
        .iter()
        .map(|n| format!("  - {n}"))
        .collect::<Vec<_>>()
        .join("\n");
    format!("missing from {missing_from}, but present in {present_in}:\n{list}")
}

#[test]
fn every_test_on_disk_is_registered_as_a_module() {
    let disk = modules_on_disk();
    let registered = modules_registered();

    let unregistered: BTreeSet<String> = disk.difference(&registered).cloned().collect();
    assert!(
        unregistered.is_empty(),
        "a test file that is not registered as a module compiles nowhere and runs \
         never, while still looking like a test that ships.\n{}\n\nAdd `mod \
         <name>;` to tests/infrastructure.rs.",
        report(
            "tests/infrastructure.rs",
            "tests/infrastructure/",
            &unregistered
        )
    );

    let orphaned: BTreeSet<String> = registered.difference(&disk).cloned().collect();
    assert!(
        orphaned.is_empty(),
        "a module is registered with no file behind it.\n{}\n\nRemove the `mod \
         <name>;` line, or restore the file.",
        report(
            "tests/infrastructure/",
            "tests/infrastructure.rs",
            &orphaned
        )
    );
}

#[test]
fn every_test_on_disk_is_accounted_for_in_the_ledger() {
    let disk = modules_on_disk();
    let ledger = modules_in_ledger();

    let undocumented: BTreeSet<String> = disk.difference(&ledger).cloned().collect();
    assert!(
        undocumented.is_empty(),
        "a test exists with no ledger row, so nothing records what it guarantees, \
         why in-process cannot show it, or which budget it spends — which is the \
         drift this guard exists to stop.\n{}\n\nAdd a row to README.md citing \
         `tests/infrastructure/<name>.rs`.",
        report("README.md", "tests/infrastructure/", &undocumented)
    );

    let stale: BTreeSet<String> = ledger.difference(&disk).cloned().collect();
    assert!(
        stale.is_empty(),
        "the ledger accounts for a test that no longer exists. A stale row is worse \
         than a missing one: it reports coverage the suite does not have.\n{}\n\n\
         Remove the row, or restore the file.",
        report("tests/infrastructure/", "README.md", &stale)
    );
}

/// The guard must not be able to pass by finding nothing on every side.
///
/// Each assertion above compares two sets, and three empty sets are equal. A
/// misresolved directory, an unreadable README or a parser that silently matches
/// nothing would satisfy every difference check while proving nothing at all —
/// the same vacuity failure this suite's schema assertions are written to avoid.
#[test]
fn the_comparison_is_not_vacuous() {
    assert!(
        !modules_on_disk().is_empty(),
        "no test files were discovered in tests/infrastructure/ — the directory \
         path is wrong, or the suite is empty"
    );
    assert!(
        !modules_registered().is_empty(),
        "no modules were parsed from tests/infrastructure.rs — the registration \
         target moved, or its syntax changed"
    );
    assert!(
        !modules_in_ledger().is_empty(),
        "no ledger rows were parsed from README.md — the ledger stopped citing \
         tests by their `tests/infrastructure/<name>.rs` path, which is the only \
         thing tying a row to a file"
    );
}

/// The anchoring rule, pinned against fixtures rather than against this
/// repository's current contents.
///
/// Every assertion above compares sets drawn from the real tree, so all of them
/// stay green if the parse is weakened in a way the current README happens to
/// tolerate. These do not: each one states the rule directly, so removing the
/// table-row check or the code-span check fails here whatever the tree looks
/// like.
mod anchoring {
    use super::ledger_citations;

    /// A Status cell: a table row, path written as a code span.
    #[test]
    fn a_status_cell_counts() {
        let found = ledger_citations(
            "| 1 | Some scenario | A guarantee | Why | `tests/infrastructure/a_test.rs` |",
        );
        assert!(
            found.contains("a_test"),
            "a path in a table row's code span is the one form that accounts for a test"
        );
    }

    /// The regression this file exists to close, stated as a rule rather than as
    /// a mutation someone once ran: prose is not a justification.
    #[test]
    fn a_prose_mention_does_not_count() {
        let found = ledger_citations(
            "It is now guarded by `tests/infrastructure/a_test.rs`; see its own docs.",
        );
        assert!(
            found.is_empty(),
            "a prose mention carries no guarantee, no why-in-process and no budget, \
             so it must not stand in for a deleted Status row — found {found:?}"
        );
    }

    /// Both anchors are required, so each is pinned on its own. Dropping either
    /// check in isolation fails exactly one of these.
    #[test]
    fn a_table_row_without_a_code_span_does_not_count() {
        let found = ledger_citations(
            "| 1 | Some scenario | A guarantee | Why | tests/infrastructure/a_test.rs |",
        );
        assert!(
            found.is_empty(),
            "a bare path in a row is not how a Status cell is written — found {found:?}"
        );
    }

    /// The exact shape that was a live false green: one document holding both a
    /// Status row and a prose mention of the same test. Deleting the row must
    /// leave nothing behind.
    #[test]
    fn deleting_a_row_is_not_rescued_by_its_prose_mention() {
        const WITH_ROW: &str = "\
| 4 | Two replicas | A guarantee | Why | `tests/infrastructure/a_test.rs` |

It is now guarded by `tests/infrastructure/a_test.rs`.
";
        const ROW_DELETED: &str = "\
It is now guarded by `tests/infrastructure/a_test.rs`.
";
        assert!(
            ledger_citations(WITH_ROW).contains("a_test"),
            "control: with its row present the test is accounted for"
        );
        assert!(
            ledger_citations(ROW_DELETED).is_empty(),
            "with the row gone the prose must not keep the test accounted for — \
             this is the false green the anchoring closes"
        );
    }
}

/// The module-registration parse, pinned the same way and for the same reason.
mod registration {
    use super::registered_modules;

    #[test]
    fn a_declaration_counts_and_a_block_header_does_not() {
        let found = registered_modules("mod infrastructure {\n    mod a_test;\n}\n");
        assert!(
            found.contains("a_test"),
            "a `mod name;` declaration registers a test"
        );
        assert!(
            !found.contains("infrastructure"),
            "the enclosing block header is not a test module — found {found:?}"
        );
    }

    #[test]
    fn a_commented_out_declaration_does_not_count() {
        let found = registered_modules("// mod a_test;\n//! mod b_test;\n");
        assert!(
            found.is_empty(),
            "a module that is commented out is registered nowhere — found {found:?}"
        );
    }
}
