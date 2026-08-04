//! Time abstractions shared across the domain layer.
//!
//! | Module | Purpose |
//! |--------|---------|
//! | [`clock`] | `Clock`/`SystemClock` — injectable UTC time source |

pub mod clock;

pub use clock::{Clock, SystemClock};
