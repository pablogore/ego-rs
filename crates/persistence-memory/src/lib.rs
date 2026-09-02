//! # ego-persistence-memory
//!
//! The single owning crate for every in-memory implementation of a
//! domain-owned persistence port (CORE-PERSIST-B). Relocated verbatim from
//! `ego-infrastructure`, `ego-testkit`, and `examples/reference-app`, whose
//! own paths keep resolving through a compatibility re-export at every old
//! location (design.md AD-6, AD-5, AD-7).

pub mod operation;
pub mod persistence;
pub mod read_side;
