# Review Policy — core-028-stage2b-service-tag-macro

- **Risk classification**: Medium (Standard tier). No security/auth/payments path, no data
  loss/exposure, no shell/process integration; authored diff ~390 lines (under the 400-line /
  hot-path threshold for full 4R).
- **Lens selected**: `review-reliability` only — dominant risk signal is behavior/tests/
  determinism (macro codegen driving trait resolution + a public API rename), matching the
  Risk table's "Behavior, state, tests, determinism, or regressions" row.
- **Target**: combined diff of `opsx/core-028-stage2b-pr1-service-tag-trait` (base `develop`)
  + stacked `opsx/core-028-stage2b-pr2-appbuilder-service`.
