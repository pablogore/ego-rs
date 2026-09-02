//! CORE-PERSIST-A IS-6/SC-1: compile-time proof that every relocated item
//! resolves to the identical item at both its old (`ego_domain::*`) and new
//! (`ego_persistence_api::*`) path — not a re-declared copy sharing a name.
//!
//! One identity witness per relocated item, grown slice by slice (S1 -> S2
//! -> S3, design.md AD-6). Object-safe traits get an identity coercion
//! (`Box<dyn old> -> Box<dyn new>`, which only type-checks against the exact
//! same trait); concrete/generic items get an identity function. Both fail
//! to compile against a re-declared same-named copy, and both fail to
//! compile before the relocation lands, which is this file's RED state.
//!
//! `ego-persistence-api` itself declares no `path` dependency on
//! `ego-domain` (SC-2/the "no workspace crate" requirement) — this file's
//! `ego-domain` dev-dependency exists only for this proof and is excluded
//! from the layer/cycle graph the same way `service-sdk`'s dev-only edge on
//! `testkit` already is (xtask/src/metadata.rs `dev_dependency_excluded_*`).

// ---- S1 — read side (design.md AD-6) ----

/// `OffsetStore` is object-safe (no generic methods) — an identity coercion
/// only type-checks if both paths name the same trait.
fn _identity_offset_store(
    x: Box<dyn ego_domain::read_side::offset::OffsetStore>,
) -> Box<dyn ego_persistence_api::read_side::offset::OffsetStore> {
    x
}

fn _identity_offset(
    x: ego_domain::read_side::offset::Offset,
) -> ego_persistence_api::read_side::offset::Offset {
    x
}

fn _identity_offset_store_error(
    x: ego_domain::read_side::offset::OffsetStoreError,
) -> ego_persistence_api::read_side::offset::OffsetStoreError {
    x
}

fn _identity_dedup_store(
    x: Box<dyn ego_domain::read_side::dedup::DedupStore>,
) -> Box<dyn ego_persistence_api::read_side::dedup::DedupStore> {
    x
}

fn _identity_dedup_store_error(
    x: ego_domain::read_side::dedup::DedupStoreError,
) -> ego_persistence_api::read_side::dedup::DedupStoreError {
    x
}

/// `ReadSideStore<E>` is generic on the trait itself, not on any method, so
/// it stays object-safe for any `E` — same coercion shape as the
/// non-generic traits above, generic over `E`.
fn _identity_read_side_store<E>(
    x: Box<dyn ego_domain::read_side::store::ReadSideStore<E>>,
) -> Box<dyn ego_persistence_api::read_side::store::ReadSideStore<E>> {
    x
}

fn _identity_read_side_store_error(
    x: ego_domain::read_side::store::ReadSideStoreError,
) -> ego_persistence_api::read_side::store::ReadSideStoreError {
    x
}

/// Old path is `read_side::projection_state_store` (KD-1: `ProjectionStateStore`
/// relocates dead, D-8); the new crate renames the file to
/// `read_side::projection_state` (task 3.4) and `ego-domain` re-exports it
/// back under the original module name.
fn _identity_projection_state_store(
    x: Box<dyn ego_domain::read_side::projection_state_store::ProjectionStateStore>,
) -> Box<dyn ego_persistence_api::read_side::projection_state::ProjectionStateStore> {
    x
}

fn _identity_projection_state_store_error(
    x: ego_domain::read_side::projection_state_store::ProjectionStateStoreError,
) -> ego_persistence_api::read_side::projection_state::ProjectionStateStoreError {
    x
}

fn _identity_event_tag(
    x: ego_domain::read_side::event_tag::EventTag,
) -> ego_persistence_api::read_side::event_tag::EventTag {
    x
}

fn _identity_projection_state(
    x: ego_domain::read_side::state::ProjectionState,
) -> ego_persistence_api::read_side::state::ProjectionState {
    x
}

fn _identity_event_stream_element<E>(
    x: ego_domain::read_side::event_stream::EventStreamElement<E>,
) -> ego_persistence_api::read_side::event_stream::EventStreamElement<E> {
    x
}
