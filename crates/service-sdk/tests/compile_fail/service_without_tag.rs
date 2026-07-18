// Fixture: a bare `#[service]` struct (no `impl_of` link) fails the real
// public macro-linked registration call, `AppBuilder::service::<S>()`
// (CORE-028 Stage 2B) — not just the underlying `HasServiceTag` bound in
// isolation, so the fixture pins the observable contract at the actual API
// surface a caller would hit.
use ego_service_sdk::app::App;
use ego_service_sdk_macros::service;

#[service]
struct UnlinkedService;

fn main() {
    let _ = App::builder().service::<UnlinkedService>();
}
