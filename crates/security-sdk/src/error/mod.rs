//! Security error types.

use thiserror::Error;

/// Unified, provider-neutral security error.
///
/// No third-party error type (e.g. `jsonwebtoken::Error`) appears in
/// this public surface. Provider failures are mapped to opaque strings.
#[derive(Debug, Error)]
pub enum SecurityError {
    /// Authentication ran but the credential was rejected.
    #[error("authentication failed: {0}")]
    AuthenticationFailed(String),

    /// The presented credential was malformed or of an unsupported scheme.
    #[error("invalid credential: {0}")]
    InvalidCredential(String),

    /// Subject identifier is invalid (must be non-empty).
    #[error("invalid subject id: {0}")]
    InvalidSubjectId(String),

    /// Authorization denied access.
    #[error("authorization denied: {reason}")]
    AuthorizationDenied {
        /// Why access was denied.
        reason: String,
    },

    /// No security context was present where one was required.
    #[error("missing security context")]
    MissingContext,

    /// Security capability is not enabled in the runtime.
    #[error("security capability not enabled")]
    CapabilityNotEnabled,

    /// A provider or backing store failed. Underlying cause is flattened to a
    /// string so no vendor type leaks through the public surface.
    #[error("provider error: {0}")]
    ProviderError(String),

    /// An access request descriptor was malformed (e.g. bad `"resource:action"` format).
    #[error("invalid access request: {0}")]
    InvalidAccessRequest(String),

    /// A caller-supplied tenant disagreed with the authoritative tenant.
    ///
    /// `Display` is deliberately redacted (AD-010 exposure boundary,
    /// NFR-003): no raw tenant identifier is interpolated, since `Display`
    /// output may reach external callers, error responses, or log sinks.
    /// `expected`/`actual` remain available as programmatic fields for
    /// `match`-based handling and appear in `Debug` for local diagnostics.
    #[error("tenant mismatch")]
    TenantMismatch {
        /// The tenant the runtime expected (e.g. `Principal.tenant_id`).
        expected: String,
        /// The tenant value actually supplied/observed.
        actual: String,
    },

    /// Cross-tenant access was requested but not authorized.
    #[error("cross-tenant access denied: {reason}")]
    CrossTenantDenied {
        /// Why cross-tenant access was denied.
        reason: String,
    },
}

#[cfg(test)]
mod tests {
    use super::SecurityError;

    #[test]
    fn display_authentication_failed() {
        let err = SecurityError::AuthenticationFailed("bad".into());
        assert!(
            err.to_string().contains("authentication failed"),
            "expected 'authentication failed', got: {}",
            err
        );
    }

    #[test]
    fn display_invalid_credential() {
        let err = SecurityError::InvalidCredential("wrong".into());
        assert!(
            err.to_string().contains("invalid credential"),
            "expected 'invalid credential', got: {}",
            err
        );
    }

    #[test]
    fn display_invalid_subject_id() {
        let err = SecurityError::InvalidSubjectId("".into());
        assert!(
            err.to_string().contains("invalid subject id"),
            "expected 'invalid subject id', got: {}",
            err
        );
    }

    #[test]
    fn display_authorization_denied() {
        let err = SecurityError::AuthorizationDenied {
            reason: "nope".into(),
        };
        assert!(
            err.to_string().contains("authorization denied"),
            "expected 'authorization denied', got: {}",
            err
        );
    }

    #[test]
    fn display_missing_context() {
        let err = SecurityError::MissingContext;
        assert!(
            err.to_string().contains("missing security context"),
            "expected 'missing security context', got: {}",
            err
        );
    }

    #[test]
    fn display_capability_not_enabled() {
        let err = SecurityError::CapabilityNotEnabled;
        assert!(
            err.to_string().contains("security capability not enabled"),
            "expected 'security capability not enabled', got: {}",
            err
        );
    }

    #[test]
    fn display_provider_error() {
        let err = SecurityError::ProviderError("io".into());
        assert!(
            err.to_string().contains("provider error"),
            "expected 'provider error', got: {}",
            err
        );
    }

    #[test]
    fn display_invalid_access_request() {
        let err = SecurityError::InvalidAccessRequest("bad".into());
        assert!(
            err.to_string().contains("invalid access request"),
            "expected 'invalid access request', got: {}",
            err
        );
    }

    #[test]
    fn no_external_type_in_variants() {
        // Compile-time check: SecurityError must be Send + Sync + 'static + std::error::Error.
        fn assert_bounds<T: std::error::Error + Send + Sync + 'static>() {}
        assert_bounds::<SecurityError>();
    }

    // TASK-001 (CORE-008A, RED): SecurityError::TenantMismatch / CrossTenantDenied.

    #[test]
    fn tenant_mismatch_display_redacts_both_identifiers() {
        let err = SecurityError::TenantMismatch {
            expected: "tenant-a".into(),
            actual: "tenant-b".into(),
        };
        let display = err.to_string();
        assert!(
            !display.contains("tenant-a") && !display.contains("tenant-b"),
            "Display MUST NOT contain either raw tenant identifier (AD-010, NFR-003), got: {}",
            display
        );
    }

    #[test]
    fn tenant_mismatch_debug_may_contain_identifiers() {
        let err = SecurityError::TenantMismatch {
            expected: "tenant-a".into(),
            actual: "tenant-b".into(),
        };
        let debug = format!("{:?}", err);
        assert!(
            debug.contains("tenant-a") && debug.contains("tenant-b"),
            "Debug MAY contain raw identifiers for internal diagnostics, got: {}",
            debug
        );
    }

    #[test]
    fn display_cross_tenant_denied() {
        let err = SecurityError::CrossTenantDenied {
            reason: "no cross-tenant capability".into(),
        };
        assert!(
            err.to_string().contains("cross-tenant access denied"),
            "expected 'cross-tenant access denied', got: {}",
            err
        );
    }
}
