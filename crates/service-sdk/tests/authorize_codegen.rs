// Trybuild tests for CORE-015 — `#[authorize]` codegen integration.
//
// compile-pass: valid annotation must produce no compile error (T-06).
// compile-fail: error diagnostics are verified by the fixtures below (T-10 – T-17).
//
// Regenerate .stderr snapshots:
//   TRYBUILD=overwrite cargo test --test authorize_codegen

#[test]
fn authorize_compile_pass() {
    let t = trybuild::TestCases::new();
    t.pass("tests/compile_pass/authorize_ok.rs");
}

#[test]
fn authorize_compile_fail() {
    let t = trybuild::TestCases::new();
    // T-10: E1 — permission literal missing ':'
    t.compile_fail("tests/compile_fail/authorize_bad_format.rs");
    // T-11: E2 — empty resource (":read")
    t.compile_fail("tests/compile_fail/authorize_empty_resource.rs");
    // T-12: E3 — empty action ("orders:")
    t.compile_fail("tests/compile_fail/authorize_empty_action.rs");
    // T-13: E_from — error type lacks From<SecurityError>
    t.compile_fail("tests/compile_fail/authorize_missing_from.rs");
    // T-14: E5 — #[authorize] outside #[service]
    t.compile_fail("tests/compile_fail/authorize_outside_service.rs");
    // T-15: E6 — context param not found in signature
    t.compile_fail("tests/compile_fail/authorize_unknown_ctx.rs");
    // T-16: E4 — unknown named argument key
    t.compile_fail("tests/compile_fail/authorize_unknown_arg.rs");
    // T-17: AD-4 non-literal — permission = SOME_CONST
    t.compile_fail("tests/compile_fail/authorize_non_literal.rs");
}
