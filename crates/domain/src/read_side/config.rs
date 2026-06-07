//! Read side projection configuration.

use serde::{Deserialize, Serialize};

/// Runtime configuration for backpressure and concurrency control.
///
/// | Field | Type | Default | Description |
/// |-------|------|---------|-------------|
/// | `batch_size` | usize | 20 | Max events per batch delivered to handler |
/// | `max_in_flight` | usize | 10 | Max concurrent batch operations globally |
/// | `concurrency_per_tag` | usize | 4 | Max concurrent tag streams per projection |
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadSideConfig {
    /// Max events per batch delivered to handler.
    pub batch_size: usize,
    /// Max concurrent batch operations globally.
    pub max_in_flight: usize,
    /// Max concurrent tag streams per projection.
    pub concurrency_per_tag: usize,
}

impl Default for ReadSideConfig {
    fn default() -> Self {
        Self {
            batch_size: 20,
            max_in_flight: 10,
            concurrency_per_tag: 4,
        }
    }
}

impl ReadSideConfig {
    /// Creates a new `ReadSideConfig` with default values.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the batch size.
    pub fn with_batch_size(mut self, batch_size: usize) -> Self {
        assert!(batch_size > 0, "batch_size must be > 0");
        self.batch_size = batch_size;
        self
    }

    /// Sets the max in-flight batches.
    pub fn with_max_in_flight(mut self, max_in_flight: usize) -> Self {
        assert!(max_in_flight > 0, "max_in_flight must be > 0");
        self.max_in_flight = max_in_flight;
        self
    }

    /// Sets the concurrency per tag.
    pub fn with_concurrency_per_tag(mut self, concurrency_per_tag: usize) -> Self {
        assert!(concurrency_per_tag > 0, "concurrency_per_tag must be > 0");
        self.concurrency_per_tag = concurrency_per_tag;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = ReadSideConfig::default();
        assert_eq!(config.batch_size, 20);
        assert_eq!(config.max_in_flight, 10);
        assert_eq!(config.concurrency_per_tag, 4);
    }

    #[test]
    fn test_builder_pattern() {
        let config = ReadSideConfig::new()
            .with_batch_size(50)
            .with_max_in_flight(20)
            .with_concurrency_per_tag(8);

        assert_eq!(config.batch_size, 50);
        assert_eq!(config.max_in_flight, 20);
        assert_eq!(config.concurrency_per_tag, 8);
    }

    #[test]
    #[should_panic(expected = "batch_size must be > 0")]
    fn test_zero_batch_size() {
        ReadSideConfig::new().with_batch_size(0);
    }

    #[test]
    #[should_panic(expected = "max_in_flight must be > 0")]
    fn test_zero_max_in_flight() {
        ReadSideConfig::new().with_max_in_flight(0);
    }

    #[test]
    #[should_panic(expected = "concurrency_per_tag must be > 0")]
    fn test_zero_concurrency_per_tag() {
        ReadSideConfig::new().with_concurrency_per_tag(0);
    }

    #[test]
    fn test_equality() {
        let c1 = ReadSideConfig::default();
        let c2 = ReadSideConfig::default();
        let c3 = ReadSideConfig::new().with_batch_size(10);
        assert_eq!(c1, c2);
        assert_ne!(c1, c3);
    }
}
