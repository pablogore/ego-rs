//! Infrastructure persistence implementations.
//!
//! Concrete backends for the domain persistence SPI traits.

pub mod in_memory;

pub use ego_persistence::postgres;
