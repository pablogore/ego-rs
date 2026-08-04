//! The `Clock` abstraction — injectable time source shared across the
//! domain layer.
//!
//! Provides a `Send + Sync` trait so that time-dependent operations (JWT
//! expiry/`nbf` checks, and any other code that would otherwise call
//! `Utc::now()` directly) can be tested deterministically. Originally scoped
//! to authentication, this abstraction now lives here as the one shared
//! home; `crate::auth::clock` re-exports the same types for existing call
//! sites, unchanged in behavior.

use chrono::{DateTime, Utc};

/// A source of the current UTC wall-clock time.
///
/// Production code should use a [`SystemClock`]; tests should inject a
/// deterministic fixed-time double.
///
/// # Invariants
///
/// - Implementations MUST be `Send + Sync` so they can be shared across
///   threads or stored inside `Arc<dyn Clock>`.
/// - Implementations SHOULD be deterministic given the same sequence of
///   `now()` calls (this is automatically true for fixed-time mocks).
pub trait Clock: Send + Sync {
    /// Returns the current UTC instant.
    fn now(&self) -> DateTime<Utc>;
}

/// A [`Clock`] backed by the real system clock (`Utc::now()`).
///
/// Use this in production. Inject a mock [`Clock`] in tests.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    /// A fixed-time clock for deterministic testing.
    struct FixedClock(DateTime<Utc>);

    impl Clock for FixedClock {
        fn now(&self) -> DateTime<Utc> {
            self.0
        }
    }

    /// `SystemClock` is the production implementation, so the only thing worth
    /// asserting about it here is structural: that it satisfies the trait, both
    /// as a generic bound and as a trait object. Reading the wall clock to check
    /// the result "looks plausible" would make this a non-deterministic test of
    /// the operating system rather than of this module, and would tie a unit
    /// test to real elapsed time. Behaviour that depends on time is tested
    /// against a controllable clock instead — see the cases below.
    #[test]
    fn system_clock_satisfies_the_clock_contract() {
        fn requires_clock<T: Clock>(_: &T) {}
        requires_clock(&SystemClock);

        let _as_trait_object: &dyn Clock = &SystemClock;
    }

    #[test]
    fn fixed_clock_returns_exact_time() {
        let ts = Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap();
        let clock = FixedClock(ts);
        assert_eq!(clock.now(), ts);
        assert_eq!(clock.now(), ts); // deterministic
    }

    #[test]
    fn clock_is_object_safe() {
        let ts = Utc.with_ymd_and_hms(2024, 3, 15, 8, 30, 0).unwrap();
        let clock: Box<dyn Clock> = Box::new(FixedClock(ts));
        assert_eq!(clock.now(), ts);
    }

    #[test]
    fn clock_works_behind_arc() {
        use std::sync::Arc;
        let ts = Utc.with_ymd_and_hms(2025, 6, 1, 12, 0, 0).unwrap();
        let clock: Arc<dyn Clock> = Arc::new(FixedClock(ts));
        assert_eq!(clock.now(), ts);
    }
}
