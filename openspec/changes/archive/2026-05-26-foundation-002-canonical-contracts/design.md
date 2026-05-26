# Design: Canonical Contract Governance

## Principles

1. **Protobuf-first.** Contracts are the source of truth. Generated code, services, and
   endpoints derive from them — never the reverse.
2. **Contract-first development.** No implementation task may depend on a public contract
   that has not been accepted through OpenSpec.
3. **Governance over implementation.** This change defines rules, structure, and processes.
   It does not implement gRPC servers, tonic services, endpoints, or `.proto` messages.

## Repository Structure

```
contracts/
├── buf.work.yaml          # Workspace definition
├── buf.yaml               # Root Buf config (lint + breaking)
└── <domain>/
    └── v1/
        ├── buf.yaml       # Module-level Buf config
        └── *.proto        # Versioned contract definitions
```

- `contracts/` is the canonical root for all protobuf contract artifacts.
- Each domain gets its own subdirectory.
- Versions are major-versioned (`v1`, `v2`, …) under each domain.
- Generated Rust code lives in a separate location (e.g., `crates/ego-rs-contracts/src/generated/`)
  and is never hand-edited.

## Buf Governance

- `buf lint` runs on every contract change.
- `buf breaking` runs against the prior accepted contract baseline.
- If the prior baseline is unavailable (first adoption), validation fails closed.
- Buf configuration lives under `contracts/` and is version-controlled.

## Versioning

- All contracts start at `v1` from first introduction.
- Incompatible changes require a new major version (`v2`, `v3`, …).
- Each incompatible version includes migration guidance in the OpenSpec change.
- Backward-compatible additions (new fields with defaults, new services) stay within the
  current major version.

## CQRS Taxonomy

Every contract is classified at proposal time:

| Classification | Meaning                        | Example                    |
| -------------- | ------------------------------ | -------------------------- |
| Command        | Requested state change         | `CreateOrder`, `CancelTx`  |
| Query          | Read operation                 | `GetOrder`, `ListUsers`    |
| Event          | Historical fact                | `OrderCreated`, `TxFailed` |

Classification drives review focus: commands require idempotency analysis, queries require
read-model impact, events require schema evolution strategy.

## Generation Policy

- Rust code is generated from accepted protobuf contracts using `prost` and `tonic`.
- Generation configuration (`build.rs`, `buf.gen.yaml`) is version-controlled and separate
  from runtime service code.
- Generated code is never hand-edited. Any required change must go through the contract or
  generation configuration.
- Generation is deterministic: same inputs produce same outputs.

## Ownership and Review

- Each contract area (domain) has an explicit owner or reviewer group.
- Review checklist:
  - Versioning and compatibility assessment
  - CQRS classification
  - Buf lint and breaking checks pass
  - Generation impact verified
  - Contract tests defined
- No contract-driven implementation begins without review approval.

## Testing Governance

- Contract tests validate schema compatibility and generation expectations.
- Tests run without live brokers, databases, network services, or external infrastructure.
- Tests use mocks or local deterministic fixtures.
- Tests comply with the project's general testing governance.

## Adoption Path

1. Create `contracts/` directory with Buf workspace and root config.
2. Define the `canonical-contract-governance` spec (this change).
3. Add the first domain contract under `contracts/<domain>/v1/`.
4. Configure prost/tonic generation for the first domain.
5. Establish ownership and review process.
