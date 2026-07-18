//! Trybuild driver for CORE-028 Stage 2B — `#[service(impl_of = Trait)]`
//! codegen / `HasServiceTag` (design.md, tasks 2.2/2.5/2.6).
//!
//! Regenerate .stderr snapshots:
//!   TRYBUILD=overwrite cargo test -p ego-service-sdk --test service_tag_codegen

#[test]
fn service_tag_codegen_compile_pass() {
    let t = trybuild::TestCases::new();
    t.pass("tests/compile_pass/service_impl_of_with_version.rs");
}

#[test]
fn service_tag_codegen_compile_fail() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/compile_fail/service_without_tag.rs");
    t.compile_fail("tests/compile_fail/service_wrong_impl_of.rs");
}
