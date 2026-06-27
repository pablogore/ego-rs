/// Zero-size compile-time proof of cross-tenant authorization.
///
/// All code within the `crate::runtime` module is considered trusted
/// infrastructure and can call `new()` directly. The documented public
/// entry point is [`RuntimeInner::issue_cross_tenant_permit`].
///
/// # Guarantee
///
/// This type cannot be forged **in safe Rust**: the constructor is
/// `pub(super)`, making it visible to all code within the `crate::runtime`
/// module, including its sibling submodules (`runtime_builder`, `builder`,
/// `resolvable`). The entire `crate::runtime` module tree is treated as
/// trusted infrastructure. The `_private` field additionally blocks
/// struct-literal construction from any module outside this file.
///
/// This guarantee applies to safe Rust only. `unsafe { std::mem::zeroed() }`
/// or `unsafe { std::mem::transmute::<(), CrossTenantPermit>(()) }` can bypass
/// it — as with all Rust type-system capability tokens. The invariant is that
/// no safe code outside `crate::runtime` can mint a permit.
///
/// # Copy + Clone design decision
///
/// `Copy + Clone` is intentional: `CrossTenantPermit` is a stateless
/// compile-time witness. Copying it is safe because the actual authorization
/// check (TASK-014) happens at issuance time inside
/// `RuntimeInner::issue_cross_tenant_permit`, not at use time. This design
/// assumes the permit is not scoped to a single context grant. If TASK-014
/// requires per-grant re-authorization, `Copy` would need to be removed
/// (a breaking change).
///
/// Compile-time gate only. TASK-014 adds the runtime authorization check.
// Copy + Clone: see "Copy + Clone design decision" in the type-level doc above.
#[derive(Debug, Copy, Clone)]
pub struct CrossTenantPermit {
    _private: (),
}

impl CrossTenantPermit {
    /// Mints a permit. Visible to all code within the `crate::runtime` module,
    /// including its sibling submodules. Callers outside `crate::runtime`
    /// cannot call this directly.
    ///
    /// Compile-time gate only. TASK-014 adds the runtime authorization check.
    // Used only in tests until TASK-014 wires up the runtime authorization check.
    #[allow(dead_code)]
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
