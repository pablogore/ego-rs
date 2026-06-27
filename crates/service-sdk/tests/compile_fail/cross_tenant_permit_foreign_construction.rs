use ego_service_sdk::CrossTenantPermit;

fn main() {
    // Path A: struct-literal construction — blocked by private field `_private`.
    let _p = CrossTenantPermit { _private: () };
}
