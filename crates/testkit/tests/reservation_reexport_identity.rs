//! CORE-PERSIST-B S2: compile-time proof that the relocated
//! `InMemoryOperationReservationStore` resolves to the identical item at
//! both its old (`ego_testkit::InMemoryOperationReservationStore`) and new
//! (`ego_persistence_memory::operation::reservation::InMemoryOperationReservationStore`)
//! path — not a re-declared copy sharing a name (design.md AD-10).
//!
//! A concrete struct, like S1's row of the matrix, gets an identity
//! coercion — it only type-checks against the exact same type.
//!
//! Fails to compile before the relocation lands: the new path does not
//! exist yet, which is this file's RED state.

fn _identity_in_memory_operation_reservation_store(
    x: ego_testkit::InMemoryOperationReservationStore,
) -> ego_persistence_memory::operation::reservation::InMemoryOperationReservationStore {
    x
}
