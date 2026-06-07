//! Projection error types.

use thiserror::Error;

/// Classification of handler failures.
///
/// | Variant | Runtime Action |
/// |---------|----------------|
/// | `Transient` | Retry batch with exponential backoff (max 3 retries, 100ms base, 10s max) |
/// | `Fatal` | Stop projection immediately, raise alert |
/// | `PoisonEvent` | Log and skip the offending event, continue processing rest of batch |
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum ProjectionError {
    /// Transient error — batch should be retried with exponential backoff.
    #[error("transient error: {0}")]
    Transient(String),

    /// Fatal error — projection should be stopped immediately.
    #[error("fatal error: {0}")]
    Fatal(String),

    /// Poison event — this specific event should be skipped, rest of batch continues.
    #[error("poison event: {0}")]
    PoisonEvent(String),
}

impl ProjectionError {
    /// Creates a new transient error.
    pub fn transient(msg: impl Into<String>) -> Self {
        Self::Transient(msg.into())
    }

    /// Creates a new fatal error.
    pub fn fatal(msg: impl Into<String>) -> Self {
        Self::Fatal(msg.into())
    }

    /// Creates a new poison event error.
    pub fn poison_event(msg: impl Into<String>) -> Self {
        Self::PoisonEvent(msg.into())
    }

    /// Returns true if this is a transient error.
    pub fn is_transient(&self) -> bool {
        matches!(self, Self::Transient(_))
    }

    /// Returns true if this is a fatal error.
    pub fn is_fatal(&self) -> bool {
        matches!(self, Self::Fatal(_))
    }

    /// Returns true if this is a poison event error.
    pub fn is_poison_event(&self) -> bool {
        matches!(self, Self::PoisonEvent(_))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_transient() {
        let err = ProjectionError::transient("connection lost");
        assert!(err.is_transient());
        assert!(!err.is_fatal());
        assert!(!err.is_poison_event());
        assert_eq!(format!("{}", err), "transient error: connection lost");
    }

    #[test]
    fn test_fatal() {
        let err = ProjectionError::fatal("data corruption");
        assert!(!err.is_transient());
        assert!(err.is_fatal());
        assert!(!err.is_poison_event());
        assert_eq!(format!("{}", err), "fatal error: data corruption");
    }

    #[test]
    fn test_poison_event() {
        let err = ProjectionError::poison_event("invalid payload");
        assert!(!err.is_transient());
        assert!(!err.is_fatal());
        assert!(err.is_poison_event());
        assert_eq!(format!("{}", err), "poison event: invalid payload");
    }

    #[test]
    fn test_equality() {
        let e1 = ProjectionError::transient("err");
        let e2 = ProjectionError::transient("err");
        let e3 = ProjectionError::fatal("err");
        assert_eq!(e1, e2);
        assert_ne!(e1, e3);
    }

    #[test]
    fn test_clone() {
        let err = ProjectionError::fatal("test");
        let cloned = err.clone();
        assert_eq!(err, cloned);
    }
}
