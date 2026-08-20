.PHONY: test test-cov clippy buf-deterministic buf lint buf

# Generated code is never hand-edited. Any changes must go through the contract or generation configuration.
test:
	cargo test --workspace
	cargo test -p security-jwt --features test-kit
	cargo test --test contract_tests

# The floor lives in scripts/verify-coverage.sh, the gate CI actually runs. This
# target is its human-readable HTML companion, so it asks that script for the
# number instead of carrying a second copy that could drift away from it.
test-cov:
	cargo tarpaulin --workspace --out Html --fail-under $$(./scripts/verify-coverage.sh --print-floor)

buf-deterministic:
	./scripts/buf generate
	git diff --exit-code crates/ego-rs-contracts/src/generated

buf:
	./scripts/buf check

clippy:
	cargo clippy --workspace -- -D warnings
