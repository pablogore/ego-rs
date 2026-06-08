//! Metrics collection for the scheduler.

/// Metrics collected by the scheduler.
#[derive(Debug, Clone, Default)]
pub struct SchedulerMetrics {
    /// Total number of events consumed.
    pub total_events_consumed: u64,
    
    /// Total number of gaps detected.
    pub total_gaps_detected: u64,
    
    /// Total number of suggestions made.
    pub total_suggestions_made: u64,
}