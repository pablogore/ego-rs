//! Stoolap-backed implementation of `ego_domain::operation::OperationReservationStore`
//! (STOOLAP-S3). Gated behind the `operation-reservation` feature (design.md
//! AD-7) so `Repository<A>`/`Snapshot`/`EventStore<E>` consumers of this
//! crate gain no new dependency.

pub mod reservation;
