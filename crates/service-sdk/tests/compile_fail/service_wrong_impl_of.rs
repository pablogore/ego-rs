// Fixture: `impl_of` naming a trait the struct does not implement fails to
// compile with an ordinary "trait not implemented" error at the generated
// `into_service` body — no custom macro diagnostic (design.md AD-2 /
// spec.md's "trait-link argument naming a trait the struct does not
// implement fails to compile" scenario).
use ego_service_sdk_macros::service;

#[service]
pub trait UnrelatedTrait {}

#[service(impl_of = UnrelatedTrait)]
struct NotImplementingStruct;

fn main() {}
