## 1. Governance Specs Validation

- [x] 1.1 Validate `architecture-governance` spec — confirm all requirements are testable with WHEN/THEN scenarios
- [x] 1.2 Validate `testing-governance` spec — confirm all requirements are testable with WHEN/THEN scenarios
- [x] 1.3 Review governance specs against `rust-cqrs-framework` change to ensure consistency

## 2. CI Enforcement Setup

- [x] 2.1 Add step in CI pipeline to verify crate dependency graph matches hexagonal layers (transport → application → domain, infrastructure → domain)
- [x] 2.2 Add step in CI pipeline to verify no test file imports real infrastructure types (database drivers, Kafka clients, HTTP clients)
- [x] 2.3 Configure `cargo-tarpaulin` with `--fail-under 95` flag in CI coverage job
- [x] 2.4 Add CI step that scans for `#[automock]` on all port traits, fails if any trait is missing mock support

## 3. Developer Tooling

- [x] 3.1 Add `#[automock]` dev-dependency to workspace — ensure `mockall` is available project-wide
- [x] 3.2 Create or document `cargo test` alias with coverage (e.g., `make test-cov`) for local development
- [x] 3.3 Add pre-commit or pre-push hook that runs `cargo clippy -- -D warnings` and `cargo test`

## 4. Documentation

- [ ] 4.1 Archive governance specs to `openspec/specs/` as permanent project standards
- [ ] 4.2 Document governance rules in project README for new contributors
- [ ] 4.3 Add checklist for spec authors: "Before submitting a spec, verify: hexagonal layers? mockable traits? 95% coverage achievable?"
