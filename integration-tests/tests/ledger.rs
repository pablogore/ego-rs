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

/// Every test the ledger accounts for, discovered by the path it cites.
///
/// The ledger cites each test as `tests/infrastructure/<name>.rs`. That path is
/// the machine-readable part of an otherwise prose document, which keeps one
/// source of truth: the table a human reads is the table this parses, rather than
/// a duplicate list that could itself drift.
fn modules_in_ledger() -> BTreeSet<String> {
    let ledger = std::fs::read_to_string(README)
        .unwrap_or_else(|e| panic!("the ledger must be readable: {e}"));

    const PREFIX: &str = "tests/infrastructure/";
    const SUFFIX: &str = ".rs";

    let mut found = BTreeSet::new();
    let mut rest = ledger.as_str();
    while let Some(start) = rest.find(PREFIX) {
        rest = &rest[start + PREFIX.len()..];
        let Some(end) = rest.find(SUFFIX) else {
            continue;
        };
        let name = &rest[..end];
        if is_module_name(name) {
            found.insert(name.to_owned());
        }
    }
    found
}

/// A Rust module name as this suite writes them: lowercase, digits, underscores.
///
/// Rejecting anything else is what stops a prose mention like
/// `tests/infrastructure/<name>.rs` in a template or an example from being
/// counted as a real entry.
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
