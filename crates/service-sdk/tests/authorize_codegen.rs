// Trybuild tests for CORE-015 — `#[authorize]` codegen integration.
//
// compile-pass: valid annotation must produce no compile error (T-06).
// compile-fail: error scenarios are covered in PR 3 (T-10 – T-17).
//
// Regenerate .stderr snapshots:
//   TRYBUILD=overwrite cargo test --test authorize_codegen

#[test]
fn authorize_compile_pass() {
    let t = trybuild::TestCases::new();
    t.pass("tests/compile_pass/authorize_ok.rs");
}
