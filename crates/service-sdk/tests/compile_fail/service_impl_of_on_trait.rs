// Fixture: `#[service(impl_of = Trait)]` is only meaningful on a `struct`
// annotation (it links the struct to the trait's resolution Tag). Writing
// `impl_of` on a `trait` annotation instead must be an explicit compile
// error, not a silently discarded macro argument.
use ego_service_sdk_macros::service;

#[service]
pub trait SomeOtherTrait {}

#[service(impl_of = SomeOtherTrait)]
pub trait MyTraitService {}

fn main() {}
