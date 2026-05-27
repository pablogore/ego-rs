## Why

Dependency relationships across hexagonal architecture layers, runtime boundaries, and workspace modules are currently implicit and insufficiently governed. Without constitutional dependency governance, architectural boundaries drift, hidden coupling emerges, dependency direction violations accumulate, and deterministic behavior becomes harder to guarantee. A dedicated Dependency Governance Constitution is needed to centralize dependency direction rules, forbidden dependencies, version governance, hidden coupling prevention, and enforcement.

## What Changes

- Create a constitutional **Dependency Governance Constitution** spec (`specs/dependency-governance-constitution/spec.md`) that defines allowed dependency directions, forbidden dependencies, dependency governance rules, version governance, workspace dependency expectations, hidden coupling prevention, and governance enforcement
- Amend the **Architecture Governance** spec to cross-reference the Dependency Governance Constitution for dependency direction governance

## Capabilities

### New Capabilities
- `dependency-governance-constitution`: Constitutional governance for dependency behavior across ego-rs. Defines allowed dependency directions, forbidden dependencies, dependency visibility, version governance, workspace dependency rules, hidden coupling prevention, and governance enforcement.

### Modified Capabilities
- `architecture-governance`: Cross-reference the Dependency Governance Constitution for governance of dependency direction rules. Architectural layer dependency direction SHALL be governed by both Architecture Governance and Dependency Governance.

## Impact

- `openspec/specs/`: New `dependency-governance-constitution/` spec directory with `spec.md`
- `openspec/specs/architecture-governance/spec.md`: Amendment to cross-reference dependency governance
- No runtime code, no package manager prescriptions, no build tooling, no dependency injection framework changes
