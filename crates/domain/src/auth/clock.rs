//! The `Clock` abstraction — injectable time source for authentication logic.
//!
//! Provides a `Send + Sync` trait so that time-dependent operations (JWT
//! expiry, `nbf` checks) can be tested deterministically without calling
//! `Utc::now()` directly inside production code.

use chrono::{DateTime, Utc};

/// A source of the current UTC wall-clock time.
///
/// Production code should use a [`SystemClock`]; tests should inject a
/// [`crate`]-local mock that returns a fixed timestamp.
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

    #[test]
    fn system_clock_returns_plausible_time() {
        let before = Utc::now();
        let got = SystemClock.now();
        let after = Utc::now();
        assert!(got >= before);
        assert!(got <= after);
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
        let clock: Box<dyn Clock> = Box::new(SystemClock);
        let _ = clock.now();
    }

    #[test]
    fn clock_works_behind_arc() {
        use std::sync::Arc;
        let ts = Utc.with_ymd_and_hms(2025, 6, 1, 12, 0, 0).unwrap();
        let clock: Arc<dyn Clock> = Arc::new(FixedClock(ts));
        assert_eq!(clock.now(), ts);
    }
}
