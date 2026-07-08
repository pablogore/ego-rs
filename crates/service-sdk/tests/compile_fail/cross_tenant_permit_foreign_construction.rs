use ego_service_sdk::CrossTenantPermit;

fn main() {
    // Path A: struct-literal construction — blocked because `destination`/
    // `issued_to` are private fields, not `pub`. Uses the real field names
    // (code-review fix: this fixture previously referenced a `_private: ()`
    // field left over from before the AD-008 rename, so it only proved a
    // nonexistent-field error, not privacy).
    let _p = CrossTenantPermit {
        destination: ego_domain::context::TenantId::new("tenant-b").unwrap(),
        issued_to: ego_security_sdk::principal::SubjectId::new("alice").unwrap(),
    };
}
