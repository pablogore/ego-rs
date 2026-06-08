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

impl SchedulerMetrics {
    /// Increment the event consumption counter.
    pub fn increment_events_consumed(&mut self) {
        self.total_events_consumed += 1;
    }
    
    /// Increment the gap detection counter.
    pub fn increment_gaps_detected(&mut self) {
        self.total_gaps_detected += 1;
    }
    
    /// Increment the suggestion counter.
    pub fn increment_suggestions_made(&mut self) {
        self.total_suggestions_made += 1;
    }
}