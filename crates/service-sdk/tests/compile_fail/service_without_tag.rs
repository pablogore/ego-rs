// Fixture: a bare `#[service]` struct (no `impl_of` link) fails the
// `HasServiceTag` bound (CORE-028 Stage 2B, design.md's marker-trait
// decision). Checked directly against the bound here — `AppBuilder::
// service::<S>()` itself is bounded on `HasServiceTag` too, but lands in a
// later, stacked PR (this PR's work unit is self-contained, no `AppBuilder`
// change).
use ego_service_sdk::runtime::HasServiceTag;
use ego_service_sdk_macros::service;

#[service]
struct UnlinkedService;

fn requires_service_tag<S: HasServiceTag>() {}

fn main() {
    requires_service_tag::<UnlinkedService>();
}
