//! Structural guard: no protocol type reaches the layers that own the
//! idempotency guarantee.
//!
//! The carrier contract's whole claim is that an adapter contributes a
//! *location* and never a *rule*: `HeaderCarrier` says where an HTTP request
//! keeps the operation key, `GrpcMetadataCarrier` says where a gRPC request
//! keeps it, and both hand the raw value to one resolver that decides what it
//! means. The claim only holds while the deciding layers stay ignorant of
//! which protocol asked. The moment a `HeaderMap`, a `MetadataMap`, or an
//! `axum`/`tonic` path appears in the domain port, in the entity runtime that
//! writes a receipt, or in a reservation store, the guarantee has acquired a
//! transport-shaped dependency — and "identical behaviour across transports"
//! stops being a property of the code and becomes a property of two authors
//! having independently agreed.
//!
//! This is a plain text scan, deliberately the same mechanism and register as
//! `crates/service-sdk/tests/tenant_scoped_lint.rs`'s
//! `runtime_tenant_enforcement_path_has_no_transport_dependency` — the same
//! `workspace_root` ascent, the same recursive `.rs` collection, the same
//! best-effort identifier list. It is a different target set applied through
//! the existing approach, not a third scanning mechanism.
//!
//! It lives in `ego-transport` on purpose: this crate is the one that owns
//! both carriers, so it is the crate whose changes are most likely to push a
//! protocol type across the boundary, and the guard belongs next to the thing
//! it guards against.
//!
//! # Not gated on the `grpc` feature
//!
//! Neutrality must hold in the default, HTTP-only build too. A guard that only
//! ran under a non-default feature would be a guard almost nobody runs, and it
//! would be silent in exactly the build most contributors compile.
//!
//! # Known limitation
//!
//! Like the scan it copies, this is an identifier-name heuristic, not a
//! dependency-graph audit. A protocol type reaching these layers through a
//! re-export under a neutral name is a false negative this cannot see. Passing
//! is evidence, never proof.

use std::path::{Path, PathBuf};

/// Identifiers that would indicate a protocol type crossing into a layer that
/// must not know which transport asked. Deliberately narrow, same best-effort
/// spirit as the `TRANSPORT_IDENTIFIERS` list this is modelled on.
const PROTOCOL_IDENTIFIERS: [&str; 6] = [
    "axum",
    "tonic",
    "HeaderMap",
    "MetadataMap",
    "grpc",
    "http::header",
];

/// Crate roots scanned in full, recursively.
///
/// `crates/domain` holds the reservation port and the receipt type themselves;
/// `crates/persistent-entity` holds the runtime that writes a receipt inside
/// the unit of work. Both are scanned as whole crates rather than as picked
/// files, because a new module added under either is exactly the drift this is
/// meant to catch and a hand-maintained file list would not see it.
const SCANNED_CRATES: [&str; 2] = ["crates/domain", "crates/persistent-entity"];

/// Individual files carrying a reservation or receipt surface that lives
/// outside the two scanned crates.
///
/// Named one by one rather than by scanning their whole crates: `service-sdk`
/// and `persistence` legitimately contain plenty that is not part of this
/// contract, and widening the scan to those crates would turn an unrelated
/// change into a failure of this test and teach people to weaken it.
const SCANNED_SURFACE_FILES: [&str; 4] = [
    // The enforcement policy that consumes the reservation port — the layer
    // that decides what a missing key means, and therefore the layer that must
    // never learn which protocol failed to send one.
    "crates/service-sdk/src/runtime/idempotency.rs",
    // Readiness for the registered reservation store.
    "crates/service-sdk/src/health/reservation_store.rs",
    // The durable reservation adapter. An adapter contributes a location, and
    // a database adapter's location is a table — never a header.
    "crates/persistence/src/postgres/reservation.rs",
    // The durable receipt write: the receipt lands in the same transaction as
    // the events, and that transaction must be describable without reference
    // to how the request arrived.
    "crates/persistence/src/postgres/event_store.rs",
];

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
                "protocol_neutrality: could not locate workspace root ascending from {}",
                start.display()
            );
        }
    }
}

/// Recursively collects every `.rs` file under `dir`.
///
/// A directory that cannot be read is fatal rather than skipped. Returning
/// quietly would drop a whole scan target while every assertion below still
/// held — over the files that *were* read — and a guard that narrows its own
/// scope on failure reports the property as intact precisely when it has
/// stopped checking for it.
fn collect_rs_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries = std::fs::read_dir(dir).unwrap_or_else(|err| {
        panic!(
            "protocol_neutrality: could not read {} ({err}) — refusing to continue, \
             because a target that was never read cannot support an absence claim",
            dir.display()
        )
    });
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_rs_files(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            out.push(path);
        }
    }
}

/// Whether `haystack` contains `ident` at an identifier boundary.
///
/// A plain `contains` would be wrong here, and provably so rather than
/// theoretically: both scanned crates describe fencing tokens and sequence
/// counters as *monotonic*, and `monotonic` contains `tonic`. Those are
/// ordinary English words in doc-comments, not a gRPC dependency, and a scan
/// that flagged them would be abandoned within a day.
///
/// The narrowest fix is a boundary rule on the left edge only: a match counts
/// unless the character before it could itself be part of an identifier. That
/// rejects `monotonic` and `Monotonic` while keeping every real form —
/// `use tonic::…`, `::tonic`, `(tonic)` — and it stays a rule about token
/// shape rather than a hardcoded exception for one word, so a future
/// `nanotonic` needs no maintenance here. The right edge is deliberately left
/// open: `axum_core`, `HeaderMapExt`, and `grpc_client` are all genuine hits
/// and must stay hits.
fn find_identifier(haystack: &str, ident: &str) -> Option<usize> {
    let mut from = 0;
    while let Some(offset) = haystack[from..].find(ident) {
        let at = from + offset;
        let preceded_by_identifier_char = haystack[..at]
            .chars()
            .next_back()
            .is_some_and(|c| c.is_alphanumeric() || c == '_');
        if !preceded_by_identifier_char {
            return Some(at);
        }
        from = at + 1;
    }
    None
}

/// Reports the 1-based line number containing byte offset `at`, so a failure
/// points at a place a reader can open rather than at a file.
fn line_of(content: &str, at: usize) -> usize {
    content[..at].bytes().filter(|b| *b == b'\n').count() + 1
}

#[test]
fn idempotency_layers_carry_no_protocol_type() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let root = workspace_root(&manifest_dir);

    let mut files = Vec::new();
    // Each crate is checked for its own contribution, not just the total. The
    // named surface files below supply plenty of content on their own, so a
    // crate that silently resolved to nothing — renamed, moved, unreadable —
    // would leave every global counter healthy while its entire contents went
    // unexamined. The per-target assertion is what makes the guard's scope
    // visible instead of implicit.
    for crate_dir in SCANNED_CRATES {
        let crate_root = root.join(crate_dir);
        assert!(
            crate_root.is_dir(),
            "protocol_neutrality: scanned crate {} is missing — if it moved, update \
             SCANNED_CRATES; leaving it stale removes that crate from the guard while \
             the test keeps passing",
            crate_root.display()
        );
        let before = files.len();
        collect_rs_files(&crate_root, &mut files);
        assert!(
            files.len() > before,
            "protocol_neutrality: scanned crate {} contributed no .rs files — the \
             absence assertion would then hold over nothing for this target",
            crate_root.display()
        );
    }
    for surface in SCANNED_SURFACE_FILES {
        let path = root.join(surface);
        assert!(
            path.is_file(),
            "protocol_neutrality: expected reservation/receipt surface {} is missing — \
             if it moved, update SCANNED_SURFACE_FILES; leaving it stale silently \
             removes that surface from the guard",
            path.display()
        );
        files.push(path);
    }

    // Vacuity backstop. Each target is now asserted individually above, which
    // is where a lost scan target is actually caught; these totals remain as a
    // last line against a root ascent that landed somewhere unexpected. A scan
    // that found nothing would report a clean pass — the most dangerous
    // possible outcome, because it looks exactly like the property holding.
    // Counting files is not enough on its own either: an empty file
    // contributes to the count and zero bytes to the check, so the byte total
    // is what proves content was actually examined.
    let mut scanned_bytes = 0usize;
    let mut scanned_files = 0usize;

    for file in &files {
        // Not `continue`: a file that could not be read is a hole in the scan,
        // and skipping it removes it from the check while leaving the totals
        // below looking healthy.
        let content = std::fs::read_to_string(file).unwrap_or_else(|err| {
            panic!(
                "protocol_neutrality: could not read {} ({err}) — refusing to treat an \
                 unread file as one that carried no protocol type",
                file.display()
            )
        });
        scanned_files += 1;
        scanned_bytes += content.len();

        for ident in PROTOCOL_IDENTIFIERS {
            if let Some(at) = find_identifier(&content, ident) {
                panic!(
                    "protocol type {ident:?} found in {} at line {} — this layer owns part of \
                     the idempotency guarantee and must never know which transport asked. \
                     A carrier contributes a location, never a rule: the moment a \
                     protocol type reaches here, the guarantee has a transport-shaped \
                     dependency and can no longer be identical across transports by \
                     construction. Move the protocol-specific part into a carrier under \
                     crates/transport and pass an already-extracted, protocol-free value in.",
                    file.display(),
                    line_of(&content, at)
                );
            }
        }
    }

    assert!(
        scanned_files > 0,
        "protocol_neutrality: scanned zero files under {} — the guard is asserting \
         absence over nothing and would pass no matter what the code does",
        root.display()
    );
    assert!(
        scanned_bytes > 0,
        "protocol_neutrality: scanned {scanned_files} files totalling zero bytes under {} — \
         every read produced empty content, so nothing was actually checked",
        root.display()
    );
}
