# Foundation 002: Canonical Contract Governance

## Problem

ego-rs has no formal governance for protobuf contracts. Without a contract-first process,
contract semantics can drift between runtime code, generated code, and service boundaries.
There is no repository structure, versioning strategy, or review model for contracts.

## Goal

Define canonical contract governance for ego-rs so that all protobuf contracts are
proposed, reviewed, versioned, and validated before they drive generated code, services,
endpoints, or runtime behavior.

## Scope

- Protobuf-first contract governance
- Contract-first development process
- Repository structure for `contracts/`
- Versioning strategy (v1 from day one)
- Buf governance (lint + breaking checks)
- prost/tonic generation policy
- CQRS contract taxonomy (commands, queries, events)
- Backward compatibility rules
- Contract ownership and review model
- Contract testing governance

## Out of Scope

- gRPC server implementation
- Tonic service implementation
- Endpoint implementation
- `.proto` implementation details
- Runtime behavior
- Speculative code generation

## Capabilities

1. **canonical-contract-governance** — Protobuf-first governance, contract-first development,
   repository structure, versioning, Buf validation, generation policy, CQRS taxonomy,
   backward compatibility, ownership/review, and testing governance.

## Impact

- Adds governance requirements to the project spec surface.
- Introduces a `contracts/` directory structure and Buf tooling configuration.
- Establishes a review gate: no contract-driven implementation without an accepted OpenSpec change.
- No runtime behavior changes.

## Risks

- Teams may resist the contract-first gate if they are accustomed to coding first.
- Buf breaking-change checks require a baseline; initial adoption needs a starting point.
- Generation policy must stay separate from runtime service code to avoid coupling.
