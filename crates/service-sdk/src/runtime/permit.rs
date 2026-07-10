use ego_domain::context::TenantId;
use ego_security_sdk::principal::SubjectId;

/// Compile-time-gated, destination-scoped proof of cross-tenant authorization
/// (CORE-008A AD-008).
///
/// All code within the `crate::runtime` module is considered trusted
/// infrastructure and can call `new()` directly. The documented public
/// entry point is [`crate::runtime::RuntimeInner::issue_cross_tenant_permit`].
///
/// # Guarantee
///
/// This type cannot be forged **in safe Rust**: the constructor is
/// `pub(super)`, making it visible to all code within the `crate::runtime`
/// module, including its sibling submodules (`runtime_builder`, `builder`,
/// `resolvable`). The entire `crate::runtime` module tree is treated as
/// trusted infrastructure. The private fields additionally block
/// struct-literal construction from any module outside this file.
///
/// This guarantee applies to safe Rust only. `unsafe { std::mem::zeroed() }`
/// or `unsafe { std::mem::transmute::<(), CrossTenantPermit>(()) }` can bypass
/// it — as with all Rust type-system capability tokens. The invariant is that
/// no safe code outside `crate::runtime` can mint a permit.
///
/// # Copy + Clone design decision (revised, AD-008)
///
/// `Copy` is **removed** as of CORE-008A Phase 4: the permit is no longer a
/// stateless, zero-size witness — it now carries the destination tenant it
/// was authorized for and the subject it was issued to. `Clone` is kept so a
/// single issued permit can still authorize multiple context grants (see
/// [`crate::context::ServiceContext::with_cross_tenant_access`]), but a
/// permit authorizing `tenant-b` cannot be silently duplicated into one that
/// authorizes `tenant-c` — `destination` travels with every clone.
///
/// The real authorization check happens once, at issuance time, inside
/// [`crate::runtime::RuntimeInner::issue_cross_tenant_permit`] (AD-008/FR-005/FR-006), not at
/// use time.
#[derive(Debug, Clone)]
pub struct CrossTenantPermit {
    destination: TenantId,
    issued_to: SubjectId,
}

impl CrossTenantPermit {
    /// Mints a permit for `destination`, authorized on behalf of `issued_to`.
    /// Visible to all code within the `crate::runtime` module, including its
    /// sibling submodules. Callers outside `crate::runtime` cannot call this
    /// directly — the only production caller is
    /// [`crate::runtime::RuntimeInner::issue_cross_tenant_permit`], which runs the
    /// `AuthorizationProvider` capability check first (AD-008).
    // Used only in tests until a real production caller adopts cross-tenant
    // issuance (this framework-stage codebase has no application services yet).
    #[allow(dead_code)]
    pub(super) fn new(destination: TenantId, issued_to: SubjectId) -> Self {
        Self { destination, issued_to }
    }

    /// The tenant this permit authorizes access to. Read by
    /// [`crate::context::ServiceContext::with_cross_tenant_access`] to scope
    /// the grant to this specific destination (AD-008 — a permit authorizing
    /// `tenant-b` cannot be reused to reach `tenant-c`).
    pub(crate) fn destination(&self) -> &TenantId {
        &self.destination
    }

    /// The subject the permit was issued to, for diagnostics/audit.
    #[allow(dead_code)]
    pub(crate) fn issued_to(&self) -> &SubjectId {
        &self.issued_to
    }
}

#[cfg(test)]
mod tests {
    use super::CrossTenantPermit;
    use ego_domain::context::TenantId;
    use ego_security_sdk::principal::SubjectId;

    fn permit(dest: &str) -> CrossTenantPermit {
        CrossTenantPermit::new(
            TenantId::new(dest).unwrap(),
            SubjectId::new("user:test").unwrap(),
        )
    }

    #[test]
    fn destination_returns_the_issued_tenant() {
        let p = permit("tenant-b");
        assert_eq!(p.destination().as_str(), "tenant-b");
    }

    #[test]
    fn clone_preserves_destination_and_issued_to() {
        let p = permit("tenant-b");
        let cloned = p.clone();
        assert_eq!(cloned.destination(), p.destination());
        assert_eq!(cloned.issued_to(), p.issued_to());
    }
}
