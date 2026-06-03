# Agent Execution Guide

Execution behavior for coding agents operating in this repository. Focused on operational rules — not architecture principles or engineering law.

## Operational Rules

- **Patch over rewrite** — extend existing modules to add functionality. Create new files only when existing structure cannot accommodate the change without violating layer rules in `docs/architecture.md`.
- **Update existing modules** — when adding functionality, modify the appropriate existing module. Do not create parallel or duplicate modules.
- **Explicit file paths** — when modifying code, reference specific file paths. Avoid vague descriptions like "update persistence layer."
- **Stop on architectural ambiguity** — if a change's effect on architecture (layering, crate boundaries, dependency direction) is unclear, stop and ask for guidance. Do not proceed based on assumptions.
- **Respect governing documents** — this file, `docs/architecture.md`, and `.speckit/constitution.md` SHALL be followed. When conflict exists, `.speckit/constitution.md` takes precedence.
- **Ignore generated/vendor/tool folders** — `node_modules/`, `target/`, `graphify-out/`, `.specs-fire/`, `.specsmd/`, `.specify/` and similar generated directories SHOULD be ignored unless evidence requires otherwise.
- **Avoid duplicate code** — check existing modules before writing new code. If similar functionality exists, extend or reuse rather than duplicate.
- **Avoid duplicate modules** — do not create modules that overlap in responsibility with existing modules in `crates/`.
- **Do not invent structure** — follow existing module organization, naming conventions, and code patterns. Do not introduce new directories, module hierarchies, or structural patterns unless required by the plan.
- **Modify before duplicate** — before creating a new module, trait, file, or abstraction, verify whether an equivalent implementation already exists. Duplication requires justification per `.speckit/constitution.md` §H.
- **Task precision** — every task MUST include: exact file path, modification type (Create/Modify/Refactor/Delete), section identifier, expected outcome, and validation criteria. Follow the format in `.speckit/constitution.md` §F.

## Governing Document Hierarchy

1. **`.speckit/constitution.md`** — engineering law (spec scope, artifact policy, task precision, WHAT vs HOW, over-engineering, escalation, testing)
2. **`docs/architecture.md`** — engineering structure (crate boundaries, design preferences, spec integration)
3. **`ARCHITECTURE.md`** — runtime architecture (layers, actors, CQRS/ES)
4. **This file** — agent execution behavior (operational rules)

## Current Plan

<!-- SPECKIT START -->
For additional context about technologies to be used, project structure,
shell commands, and other important information, read the current design:
`specs/002-correlation-lifecycle-contract/plan.md`
<!-- SPECKIT END -->
