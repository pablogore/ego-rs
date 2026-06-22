//! Authorization decision type.

/// Outcome of an authorization evaluation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthorizationDecision {
    /// Access is granted.
    Allow,
    /// Access is denied.
    ///
    /// # Stability contract
    ///
    /// `reason` is a **non-contractual, human-readable string** intended for
    /// logging and debugging. Consumers **must not** branch on specific reason
    /// strings — they are not stable across versions and differ between
    /// providers.
    ///
    /// # Future work — typed denial reasons
    ///
    /// A structured variant is planned (tracked as a future improvement):
    ///
    /// ```text
    /// enum DenialReason {
    ///     MissingRole,
    ///     MissingPermission,
    ///     MissingContext,
    ///     ProviderFailure,
    ///     Custom(String),
    /// }
    /// ```
    ///
    /// Until then, treat `reason` as an opaque diagnostic string.
    Deny {
        /// Non-contractual, human-readable description of why access was
        /// denied. Do not branch on specific values.
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

    #[test]
    fn deny_reason_is_informational_not_contractual() {
        // Two providers can produce different reason strings for the same
        // condition — consumers must not branch on specific values.
        let a = AuthorizationDecision::Deny { reason: "missing role".into() };
        let b = AuthorizationDecision::Deny { reason: "role missing".into() };
        assert!(!a.is_allowed());
        assert!(!b.is_allowed());
        // Both are semantically identical denials; only is_allowed() is stable.
    }
}
