//! Authorization decision type.

/// Outcome of an authorization evaluation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthorizationDecision {
    /// Access is granted.
    Allow,
    /// Access is denied with a human-readable reason.
    Deny {
        /// Why access was denied.
        reason: String,
    },
}

impl AuthorizationDecision {
    /// Returns `true` for [`Allow`][AuthorizationDecision::Allow].
    pub fn is_allowed(&self) -> bool {
        matches!(self, Self::Allow)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allow_variant_is_allowed() {
        assert!(AuthorizationDecision::Allow.is_allowed());
    }

    #[test]
    fn deny_variant_is_not_allowed() {
        assert!(!AuthorizationDecision::Deny { reason: "x".into() }.is_allowed());
    }

    #[test]
    fn deny_reason_accessible() {
        match (AuthorizationDecision::Deny { reason: "forbidden".into() }) {
            AuthorizationDecision::Deny { reason } => assert_eq!(reason, "forbidden"),
            _ => panic!("expected Deny"),
        }
    }
}
