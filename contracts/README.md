# Canonical Contract Governance

## Repository Structure

```
contracts/
├── buf.work.yaml          # Buf workspace definition
├── buf.yaml               # Root Buf config (lint + breaking)
└── <domain>/
    └── v1/
        ├── buf.yaml       # Module-level Buf config
        └── *.proto        # Versioned contract definitions
```

## Generation Output

Generated Rust code is produced by `buf generate` and written to:

```
crates/ego-rs-contracts/src/generated/
```

This path is configured in `buf.gen.yaml` (see generation policy tasks). Generated code
is never hand-edited. Any required change must go through the contract or generation
configuration.

## Governance

All protobuf contracts follow a contract-first process:

1. Propose a contract change via OpenSpec.
2. Classify the contract (Command, Query, or Event).
3. Review for backward compatibility and CQRS classification.
4. Run `make buf` to validate lint and breaking checks.
5. Generate code and commit alongside the accepted change.

See `design.md` for full governance rules.
