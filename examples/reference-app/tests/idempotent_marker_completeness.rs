//! Every mutating operation this application publishes carries `#[idempotent]`.
//!
//! # The gap this closes
//!
//! The `#[service]` generator cannot tell a mutating operation from a read-only
//! one. Nothing in the type system distinguishes "registers a user" from "reads a
//! projection", so whether an operation needs a reservation is a judgement only a
//! person can make — and a judgement nobody records is a judgement nobody can
//! check. The failure mode is silent and expensive: an operation everyone believes
//! is retry-safe, with nothing reserving, replaying, or refusing its retries.
//!
//! So the judgement is written down here, as an inventory, and this file holds it
//! to the generated contract.
//!
//! # What is checked against a fact, and what is checked against text
//!
//! Worth separating, because the two are not equally strong.
//!
//! **The marker itself is read from the generated contract**, not from source.
//! `ServiceContract::operations()` reports what the macro actually produced, so
//! `descriptor.idempotent` is the real answer to "is this operation governed by a
//! reservation". A grep for the attribute would pass on an attribute that failed to
//! expand.
//!
//! **Operation-level completeness is also a fact.** The inventory is compared to the
//! contract in both directions, so adding an operation to an existing trait fails
//! this test until somebody classifies it, and deleting one fails until the stale
//! entry goes.
//!
//! **Trait-level completeness is a text tripwire, and that is a real limitation.**
//! Nothing in the runtime enumerates registered services, so this test cannot
//! discover a brand-new `#[service]` trait on its own. It counts `#[service`
//! occurrences under `src/` and requires that count to match the number of traits
//! the inventory covers. That is a correlation, not the fact — a trait declared
//! somewhere this scan does not reach would slip past. It is here because the
//! alternative is a silent gap: without it, a whole new service could be published
//! with unmarked mutating operations and every assertion below would still pass.
//!
//! Closing that properly needs a registration mechanism the operations enrol
//! themselves in. That is a larger change than recording the judgement, and this
//! file is deliberately the smaller one.

use std::fs;
use std::path::Path;

use ego_service_sdk::contract::{OperationDescriptor, ServiceContract};
use reference_app::application::RegisterUserTag;

/// Whether an operation changes state, and therefore whether retrying it twice
/// must be prevented.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Effect {
    /// Changes state. A retry must not execute twice, so it needs the marker.
    Mutating,
    /// Reads only. A retry is harmless, so a reservation would cost durability
    /// writes for nothing.
    ///
    /// Unconstructed today, because this application publishes one operation and it
    /// mutates. Kept rather than deleted: it is half the vocabulary the inventory
    /// needs, and the assertion it drives — a read-only operation must *not* carry
    /// the marker — is the one that keeps the classification honest instead of
    /// letting `Mutating` become a rubber stamp. Deleting it would mean the next
    /// read-only operation has nowhere to be recorded.
    #[allow(dead_code)]
    ReadOnly,
}

/// One published service trait and the classification of each of its operations.
struct Inventory {
    /// The trait as it appears in source, for failure messages that point at the
    /// place a reader has to go.
    trait_name: &'static str,
    operations: &'static [(&'static str, Effect)],
    /// The generated contract for that trait.
    contract: fn() -> Vec<OperationDescriptor>,
}

/// The judgement, recorded. Adding an operation without adding it here fails the
/// completeness check below; adding it here with the wrong classification fails the
/// marker check.
fn inventory() -> Vec<Inventory> {
    vec![Inventory {
        trait_name: "RegisterUser",
        operations: &[("register", Effect::Mutating)],
        contract: || <RegisterUserTag as ServiceContract>::operations(),
    }]
}

#[test]
fn every_mutating_operation_carries_the_idempotent_marker() {
    for service in inventory() {
        let descriptors = (service.contract)();

        for (name, effect) in service.operations {
            let descriptor = descriptors
                .iter()
                .find(|op| &op.name == name)
                .unwrap_or_else(|| {
                    panic!(
                        "`{}::{name}` is in this test's inventory but not in the \
                         generated contract. Either the operation was renamed or \
                         removed and the entry is stale, or the contract is not \
                         reporting it.",
                        service.trait_name
                    )
                });

            match effect {
                Effect::Mutating => assert!(
                    descriptor.idempotent,
                    "`{}::{name}` is classified as mutating but its generated \
                     contract reports idempotent = false. A retry of it would \
                     execute twice: nothing reserves the operation, nothing \
                     replays a completed one, and nothing refuses a conflicting \
                     one. Add `#[idempotent]` to it, or reclassify it here if it \
                     genuinely only reads.",
                    service.trait_name
                ),
                Effect::ReadOnly => assert!(
                    !descriptor.idempotent,
                    "`{}::{name}` is classified as read-only but carries \
                     `#[idempotent]`. That is not harmless — the operation now \
                     takes a durable reservation on every call. Either the \
                     classification here is stale, or the marker should go.",
                    service.trait_name
                ),
            }
        }
    }
}

#[test]
fn the_inventory_lists_every_operation_the_contract_publishes() {
    for service in inventory() {
        let published: Vec<String> = (service.contract)()
            .into_iter()
            .map(|op| op.name.to_string())
            .collect();

        for name in &published {
            assert!(
                service.operations.iter().any(|(listed, _)| listed == name),
                "`{}::{name}` is published by the generated contract and absent \
                 from this test's inventory, so nobody has recorded whether it \
                 mutates state. Classify it as Mutating or ReadOnly. This is the \
                 check that stops a new operation shipping without that \
                 judgement — it is not a formality.",
                service.trait_name
            );
        }

        for (listed, _) in service.operations {
            assert!(
                published.iter().any(|name| name == listed),
                "`{}::{listed}` is in this test's inventory but the contract does \
                 not publish it. A stale entry hides a real one: the loop above \
                 would keep passing while an operation of that name no longer \
                 exists.",
                service.trait_name
            );
        }
    }
}

/// Counts `#[service` declarations under a directory.
///
/// Matches only lines whose first non-whitespace characters are `#[service`, which
/// keeps a commented-out attribute (`// #[service(...)]`) and a mention inside prose
/// from inflating the count. A spurious count fails loudly rather than silently, but
/// a test that cries wolf gets deleted, so the cheap precision is worth taking.
///
/// Still deliberately naive, and named for what it is: it establishes nothing about
/// any operation's marker.
fn count_service_declarations(dir: &Path) -> usize {
    let mut found = 0;
    for entry in fs::read_dir(dir).expect("the source directory is readable") {
        let path = entry.expect("a readable directory entry").path();
        if path.is_dir() {
            found += count_service_declarations(&path);
        } else if path.extension().is_some_and(|e| e == "rs") {
            let text = fs::read_to_string(&path).expect("a readable source file");
            found += text
                .lines()
                .filter(|line| line.trim_start().starts_with("#[service"))
                .count();
        }
    }
    found
}

/// The tripwire on the inventory's own completeness.
///
/// Its weakness is stated in this file's header rather than hidden: it compares a
/// source-text count to a hand-maintained list, so it correlates with trait-level
/// completeness instead of establishing it. The alternative is no check at all,
/// which is how a new service ships with unmarked mutating operations while every
/// other assertion here stays green.
#[test]
fn the_inventory_covers_every_service_trait_declared_in_this_application() {
    let declared = count_service_declarations(Path::new("src"));
    let covered = inventory().len();

    assert_eq!(
        declared, covered,
        "this application declares {declared} `#[service]` trait(s) under `src/` \
         and this test's inventory covers {covered}. A trait nobody added here can \
         publish mutating operations with no marker and no test would notice. Add \
         it to `inventory()` with each of its operations classified.\n\
         \n\
         If this fired because a `#[service]` moved rather than appeared, the \
         count is doing its job badly but the fix is the same: make the inventory \
         match what is published."
    );
}
