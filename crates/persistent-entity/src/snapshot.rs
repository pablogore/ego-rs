pub trait SnapshotStrategy: Send + Sync {
    fn should_snapshot(&self, version: u64) -> bool;
    fn clone_boxed(&self) -> Box<dyn SnapshotStrategy>;
}

pub struct SnapshotEveryN {
    interval: u64,
}

impl SnapshotEveryN {
    pub fn new(interval: u64) -> Self {
        SnapshotEveryN { interval }
    }
}

impl SnapshotStrategy for SnapshotEveryN {
    fn should_snapshot(&self, version: u64) -> bool {
        version > 0 && version % self.interval == 0
    }

    fn clone_boxed(&self) -> Box<dyn SnapshotStrategy> {
        Box::new(SnapshotEveryN::new(self.interval))
    }
}

pub struct NoSnapshot;

impl SnapshotStrategy for NoSnapshot {
    fn should_snapshot(&self, _version: u64) -> bool {
        false
    }

    fn clone_boxed(&self) -> Box<dyn SnapshotStrategy> {
        Box::new(NoSnapshot)
    }
}

impl Default for NoSnapshot {
    fn default() -> Self {
        NoSnapshot
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_snapshot_every_n() {
        let strategy = SnapshotEveryN::new(100);
        assert!(!strategy.should_snapshot(0));
        assert!(!strategy.should_snapshot(99));
        assert!(strategy.should_snapshot(100));
        assert!(strategy.should_snapshot(200));
        assert!(!strategy.should_snapshot(101));
    }

    #[test]
    fn test_no_snapshot() {
        let strategy = NoSnapshot;
        assert!(!strategy.should_snapshot(0));
        assert!(!strategy.should_snapshot(100));
        assert!(!strategy.should_snapshot(9999));
    }

    #[test]
    fn test_clone_boxed() {
        let strategy = SnapshotEveryN::new(100);
        let cloned = strategy.clone_boxed();
        assert!(cloned.should_snapshot(100));
        assert!(!cloned.should_snapshot(99));
    }
}
