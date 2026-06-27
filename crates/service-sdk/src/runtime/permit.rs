/// Zero-size compile-time proof of cross-tenant authorization.
///
/// The only mint point is [`RuntimeInner::issue_cross_tenant_permit`].
/// This type is unforgeable: the constructor is `pub(super)` (reachable only
/// within `crate::runtime` and its descendants), and the `_private` field
/// blocks struct-literal construction from any module outside this one.
///
/// Compile-time gate only. TASK-014 adds the runtime authorization check.
#[derive(Debug)]
pub struct CrossTenantPermit {
    _private: (),
}

impl CrossTenantPermit {
    /// Mints a permit. Reachable only within `crate::runtime` and its
    /// descendants — callers outside that scope cannot call this directly.
    ///
    /// Compile-time gate only. TASK-014 adds the runtime authorization check.
    pub(super) fn new() -> Self {
        Self { _private: () }
    }
}

#[cfg(test)]
mod tests {
    use super::CrossTenantPermit;

    #[test]
    fn cross_tenant_permit_is_zero_size() {
        assert_eq!(std::mem::size_of::<CrossTenantPermit>(), 0);
    }
}
