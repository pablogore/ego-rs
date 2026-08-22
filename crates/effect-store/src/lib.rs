//! Durable `EffectStateStore`/`EffectDedupStore` providers (PROD-002).
//!
//! Ships two day-zero durable providers behind Cargo features — `postgres`
//! (`PostgresEffectStore`) and `stoolap` (`StoolapEffectStore`) — each
//! implementing both `ego_runtime::effects::store::EffectStateStore` and
//! `EffectDedupStore` independently, per its own declared
//! [`ego_runtime::effects::store::EffectStoreCapabilities`] profile
//! (design.md §3.2). No default backend feature is enabled: a deployment
//! opts into exactly the driver it runs.

#[cfg(feature = "postgres")]
mod postgres;

#[cfg(feature = "stoolap")]
mod stoolap;

/// Shared conformance harness, public so `crates/integration-tests` can run
/// it against `PostgresEffectStore` (a real-Postgres test cannot live in
/// this crate — see `ego-rs-testing`).
pub mod conformance;

#[cfg(feature = "stoolap")]
pub use stoolap::StoolapEffectStore;

#[cfg(feature = "postgres")]
pub use postgres::{PostgresEffectStore, PostgresRetentionFailure};
