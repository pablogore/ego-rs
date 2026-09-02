//! In-memory implementations of `ego-persistence-api`'s `read_side` ports —
//! `ReadSideStore` — relocated verbatim from `ego-infrastructure`
//! (design.md AD-3, AD-4).

pub mod dedup;
pub mod offset;
pub mod store;
