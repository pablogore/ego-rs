use std::time::Instant;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LifecycleState {
    Recovering,
    Active,
    Passivating,
    Passivated,
    Failed,
}

#[derive(Debug, Clone)]
pub struct LifecycleStateMachine {
    state: LifecycleState,
    entered_active_at: Option<Instant>,
}

impl LifecycleStateMachine {
    pub fn new() -> Self {
        LifecycleStateMachine {
            state: LifecycleState::Recovering,
            entered_active_at: None,
        }
    }

    pub fn state(&self) -> LifecycleState {
        self.state
    }

    pub fn transition_to(&mut self, new: LifecycleState) -> Result<(), LifecycleState> {
        let allowed = match (self.state, new) {
            (LifecycleState::Recovering, LifecycleState::Active) => true,
            (LifecycleState::Recovering, LifecycleState::Failed) => true,
            (LifecycleState::Active, LifecycleState::Passivating) => true,
            (LifecycleState::Active, LifecycleState::Failed) => true,
            (LifecycleState::Passivating, LifecycleState::Passivated) => true,
            (LifecycleState::Passivating, LifecycleState::Failed) => true,
            _ => false,
        };
        if allowed {
            self.state = new;
            if new == LifecycleState::Active {
                self.entered_active_at = Some(Instant::now());
            }
            Ok(())
        } else {
            Err(self.state)
        }
    }

    pub fn can_accept_commands(&self) -> bool {
        matches!(self.state, LifecycleState::Active | LifecycleState::Recovering | LifecycleState::Passivating)
    }

    pub fn should_passivate(&self, timeout: std::time::Duration) -> bool {
        match (self.state, self.entered_active_at) {
            (LifecycleState::Active, Some(entered)) => entered.elapsed() >= timeout,
            _ => false,
        }
    }
}

impl Default for LifecycleStateMachine {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn test_initial_state() {
        let lsm = LifecycleStateMachine::new();
        assert_eq!(lsm.state(), LifecycleState::Recovering);
        assert!(!lsm.should_passivate(Duration::from_secs(60)));
    }

    #[test]
    fn test_transition_recovering_to_active() {
        let mut lsm = LifecycleStateMachine::new();
        assert!(lsm.transition_to(LifecycleState::Active).is_ok());
        assert_eq!(lsm.state(), LifecycleState::Active);
        assert!(lsm.can_accept_commands());
    }

    #[test]
    fn test_transition_recovering_to_failed() {
        let mut lsm = LifecycleStateMachine::new();
        assert!(lsm.transition_to(LifecycleState::Failed).is_ok());
        assert_eq!(lsm.state(), LifecycleState::Failed);
        assert!(!lsm.can_accept_commands());
    }

    #[test]
    fn test_transition_active_to_passivating() {
        let mut lsm = LifecycleStateMachine::new();
        lsm.transition_to(LifecycleState::Active).unwrap();
        assert!(lsm.transition_to(LifecycleState::Passivating).is_ok());
        assert!(lsm.can_accept_commands());
    }

    #[test]
    fn test_transition_passivating_to_passivated() {
        let mut lsm = LifecycleStateMachine::new();
        lsm.transition_to(LifecycleState::Active).unwrap();
        lsm.transition_to(LifecycleState::Passivating).unwrap();
        assert!(lsm.transition_to(LifecycleState::Passivated).is_ok());
        assert!(!lsm.can_accept_commands());
    }

    #[test]
    fn test_forbidden_passivating_to_active() {
        let mut lsm = LifecycleStateMachine::new();
        lsm.transition_to(LifecycleState::Active).unwrap();
        lsm.transition_to(LifecycleState::Passivating).unwrap();
        assert!(lsm.transition_to(LifecycleState::Active).is_err());
    }

    #[test]
    fn test_forbidden_passivated_to_active() {
        let mut lsm = LifecycleStateMachine::new();
        lsm.transition_to(LifecycleState::Active).unwrap();
        lsm.transition_to(LifecycleState::Passivating).unwrap();
        lsm.transition_to(LifecycleState::Passivated).unwrap();
        assert!(lsm.transition_to(LifecycleState::Active).is_err());
    }

    #[test]
    fn test_should_passivate_after_timeout() {
        let mut lsm = LifecycleStateMachine::new();
        lsm.transition_to(LifecycleState::Active).unwrap();
        // entered_active_at is set to now, so should_passivate with 0s timeout should be true
        assert!(lsm.should_passivate(Duration::from_secs(0)));
    }

    #[test]
    fn test_should_not_passivate_before_timeout() {
        let mut lsm = LifecycleStateMachine::new();
        lsm.transition_to(LifecycleState::Active).unwrap();
        assert!(!lsm.should_passivate(Duration::from_secs(9999)));
    }
}
