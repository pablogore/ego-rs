## Context

Contracts between runtime boundaries, actors, persistence, replay, observability, and architectural layers are currently implicit or governed by individual specs without a unified contract constitution. `runtime-abstraction` defines SPI ports (Execution, Clock, Context, Backpressure) but does not govern contract semantics. `architecture-governance` defines ports/adapters but does not define contract compatibility or evolution rules.

Key constraints:
- No serialization implementation, no schema technologies, no transport protocols
- Must remain implementation-agnostic and runtime-neutral
- Must cross-reference existing specs without duplicating their requirements
- Must align with the Determinism Constitution's severity classification model

## Goals / Non-Goals

**Goals:**
- Define canonical contract definition as a constitutional invariant
- Define deterministic contract semantics with unambiguous interpretation
- Define compatibility governance (backward, forward, deprecation, fail-closed)
- Define replay-safe contract behavior
- Define contract evolution governance with explicit expectations
- Define validation expectations and governance enforcement with four severity levels
- Define contract observability semantics
- Amend `runtime-abstraction` and `architecture-governance` to cross-reference the new spec

**Non-Goals:**
- Implementing serialization, schema technologies, or transport protocols
- Prescribing JSON, protobuf, Avro, OpenAPI, or equivalent formats
- Prescribing persistence schema technologies or tooling
- Implementing runtime messaging
- Duplicating existing governance from `runtime-abstraction` or `architecture-governance`

## Decisions

**Decision 1: Dedicated canonical contracts spec vs. extending existing specs**
- Approach: Create a standalone `canonical-contracts-constitution` spec
- Rationale: Contract governance is cross-cutting (runtime, architecture, persistence, replay, observability). A single spec provides unified governance. Existing specs cross-reference rather than duplicate.
- Alternatives considered: Embedding into `runtime-abstraction` (would conflate runtime concerns with contract governance), distributing across all specs (fragmented and inconsistent)

**Decision 2: Four-level governance severity model**
- Approach: Constitutional violation, Validation failure, Non-conformant behavior, Incomplete change
- Rationale: Aligns with the Determinism Constitution's three-level model and adds Incomplete change for missing compatibility/migration metadata, which is a distinct class of governance failure
- Alternatives considered: Using only three levels (doesn't capture missing metadata separately from behavioral violations)

**Decision 3: Constitutional vs. technical language for contracts**
- Approach: Define contracts in terms of semantic meaning, observable intent, and deterministic interpretation — never in terms of wire formats, schema languages, or API styles
- Rationale: The user explicitly requires implementation-agnostic, constitutional language. Avoids coupling to specific technologies.

## Risks / Trade-offs

- **[Scope creep into serialization/schemas]** → Clear non-goals and constitutional review gate prevent technology-specific concerns from entering the spec
- **[Overlap with Determinism Constitution]** → Determinism governs execution behavior; Canonical Contracts governs contract semantics. They are complementary but distinct. Cross-references prevent duplication.
- **[Cross-reference brittleness]** → Spec names are stable constitutional identifiers. Archive workflow resolves cross-references at archive time.
