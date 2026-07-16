//! CORE-018 aggregates (AD-4/AD-6): each `PersistentEntity` owns its own
//! Command/Event/State cleanly, no shared event enum across aggregates.

pub mod pricing;
pub mod tenant_org;
pub mod user;
