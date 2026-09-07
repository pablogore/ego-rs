.PHONY: test test-cov clippy buf-deterministic buf lint buf gate shipwright-bin shipwright-clean

SHIPWRIGHT_VERSION  ?= latest
SHIPWRIGHT_BIN_DIR  := bin
SHIPWRIGHT_BIN      := $(SHIPWRIGHT_BIN_DIR)/shipwright
SHIPWRIGHT_OS       := $(shell uname -s | tr '[:upper:]' '[:lower:]')
SHIPWRIGHT_ARCH_RAW := $(shell uname -m)
SHIPWRIGHT_ARCH     := $(if $(filter x86_64,$(SHIPWRIGHT_ARCH_RAW)),amd64,$(if $(filter aarch64,$(SHIPWRIGHT_ARCH_RAW)),arm64,$(SHIPWRIGHT_ARCH_RAW)))

export PATH := $(CURDIR)/$(SHIPWRIGHT_BIN_DIR):$(PATH)

# Generated code is never hand-edited. Any changes must go through the contract or generation configuration.
test:
	cargo test --workspace
	cargo test -p security-jwt --features test-kit
	cargo test --test contract_tests

test-cov:
	cargo tarpaulin --workspace --out Html --fail-under 95

buf-deterministic:
	./scripts/buf generate
	git diff --exit-code crates/ego-rs-contracts/src/generated

buf:
	./scripts/buf check

clippy:
	cargo clippy --workspace -- -D warnings

# Downloads the shipwright release binary for this OS/arch into
# $(SHIPWRIGHT_BIN_DIR) (prepended to PATH above) if not already present.
# Override SHIPWRIGHT_VERSION to pin a specific tag instead of latest.
#
# CAVEAT: the latest tagged release (v0.8.0) predates the rust-command
# provider and its Docker-in-Docker AsService fix (PR #240/#241) — it runs
# workspace-lint fine, but production-integration (docker: true) will fail
# against it. Until a release ships that commit or later, `make gate` only
# fully passes against a shipwright built from develop — see CONTRIBUTING.md.
shipwright-bin:
	@mkdir -p $(SHIPWRIGHT_BIN_DIR)
	@if [ ! -x "$(SHIPWRIGHT_BIN)" ]; then \
		echo "==> downloading shipwright $(SHIPWRIGHT_VERSION) ($(SHIPWRIGHT_OS)/$(SHIPWRIGHT_ARCH))"; \
		if [ "$(SHIPWRIGHT_VERSION)" = "latest" ]; then \
			url="https://github.com/pablogore/shipwright/releases/latest/download/shipwright-$(SHIPWRIGHT_OS)-$(SHIPWRIGHT_ARCH)"; \
		else \
			url="https://github.com/pablogore/shipwright/releases/download/$(SHIPWRIGHT_VERSION)/shipwright-$(SHIPWRIGHT_OS)-$(SHIPWRIGHT_ARCH)"; \
		fi; \
		curl -fL -o "$(SHIPWRIGHT_BIN)" "$$url"; \
		chmod +x "$(SHIPWRIGHT_BIN)"; \
	fi

# The complete Production Gate, run exactly as CI runs it: a single
# Shipwright invocation over .shipwright/workflow.yaml, no -step selection —
# Shipwright's own graph decides what runs (workspace-check, workspace-tests,
# workspace-lint, architecture-layers, architecture-isolation,
# repository-hygiene, production-integration). Same entrypoint locally and in
# CI (.github/workflows/shipwright-validation.yml). Needs Docker running and
# the Dagger CLI on PATH.
gate: shipwright-bin
	dagger run $(SHIPWRIGHT_BIN) --workflow .shipwright/workflow.yaml

shipwright-clean:
	rm -rf $(SHIPWRIGHT_BIN_DIR)
