# Tasks: Foundation 002 — Canonical Contract Governance

## 1. Repository structure

- [x] 1.1 Create `contracts/` directory at repository root
- [x] 1.2 Add `buf.work.yaml` workspace definition under `contracts/`
- [x] 1.3 Add `buf.yaml` root Buf configuration under `contracts/`
- [x] 1.4 Create first domain directory `contracts/<domain>/v1/`
- [x] 1.5 Add module-level `buf.yaml` under `contracts/<domain>/v1/`
- [x] 1.6 Document generation output location separate from `contracts/` (e.g., `crates/ego-rs-contracts/src/generated/`)

## 2. Buf governance

- [x] 2.1 Configure `buf lint` rules in `buf.yaml`
- [x] 2.2 Configure `buf breaking` against a baseline file (e.g., `buf.lock` or prior version snapshot)
- [x] 2.3 Add CI step or script that runs `buf lint` and `buf breaking` on contract changes
- [x] 2.4 Ensure missing baseline fails closed (no skip or bypass)

## 3. Generation policy

- [x] 3.1 Create generation configuration (e.g., `buf.gen.yaml`) for prost/tonic
- [x] 3.2 Add generation step to build pipeline
- [x] 3.3 Document that generated code is never hand-edited
- [x] 3.4 Verify generation is deterministic (same inputs produce same outputs)

## 4. CQRS taxonomy

- [x] 4.1 Document CQRS classification rules (command, query, event)
- [x] 4.2 Add classification field to contract review checklist
- [x] 4.3 Add classification guidance to contribution process

## 5. Backward compatibility

- [x] 5.1 Document backward compatibility rules
- [x] 5.2 Add compatibility checklist to review process
- [x] 5.3 Define migration guidance template for incompatible changes

## 6. Ownership and review

- [x] 6.1 Define contract ownership model (domain-level owners)
- [x] 6.2 Create contract review checklist
- [x] 6.3 Document review gate: no contract-driven implementation without approval

## 8. Buf lifecycle integration

- [x] 8.1 Create `scripts/buf` shell script with subcommands: `lint`, `breaking`, `generate`, `check` (all three)
- [x] 8.2 `buf lint` — runs `buf lint` against `contracts/`, fails closed
- [x] 8.3 `buf breaking` — runs `buf breaking --against buf.lock`, fails closed if baseline missing
- [x] 8.4 `buf generate` — runs `buf generate` using `buf.gen.yaml`, outputs to configured crate path
- [x] 8.5 `buf check` — runs lint + breaking + generate in sequence
- [x] 8.6 Add `buf` target to `Makefile` (`.PHONY: buf`)
- [ ] 8.7 Integrate `make buf` into pre-push hook (after clippy, before tests)
- [ ] 8.8 Add `buf.lock` to `.gitignore`? No — commit `buf.lock` for reproducible breaking checks
- [ ] 8.9 Document `scripts/buf` usage in CONTRIBUTING.md

## 9. Testing governance

- [ ] 9.1 Define contract testing requirements (schema compatibility, generation expectations)
- [ ] 9.2 Document infrastructure constraints (no live brokers, databases, network)
- [ ] 9.3 Add contract test expectations to review checklist
