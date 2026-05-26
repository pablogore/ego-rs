.PHONY: test test-cov clippy buf-deterministic buf lint buf

# Generated code is never hand-edited. Any changes must go through the contract or generation configuration.
test:
	cargo test --workspace

test-cov:
	cargo tarpaulin --workspace --out Html --fail-under 95

buf-deterministic:
	./scripts/buf generate
	git diff --exit-code crates/ego-rs-contracts/src/generated

buf:
	./scripts/buf check

clippy:
	cargo clippy --workspace -- -D warnings
