## Why

ego-rs lacks a constitutional definition for examples. Examples currently have no mandatory policy, no architectural compliance requirements, no testing obligations, and no governance. This means onboarding is inconsistent, architectural drift goes undetected in example code, and the platform lacks executable demonstrations of its own capabilities. A constitutional examples policy is needed to ensure every major capability is demonstrable, onboarding is practical, and examples remain production-oriented and architecture-compliant.

## What Changes

- Create a constitutional **Examples Constitution** spec (`specs/examples-constitution/spec.md`) that defines mandatory policies, categories, architecture compliance, testing, documentation, repository structure, and CI governance for all examples
- Introduce canonical example directory structure under `examples/` with mandatory categories
- Amend the **Project Constitution** to add a constitutional requirement that examples are mandatory and governed by the Examples Constitution
- Amend the **Testing Governance** spec to extend coverage requirements to example code
- Amend the **Architecture Governance** spec to extend architecture compliance validation to examples
- Amend the **Runtime Abstraction** spec to require example ports demonstrate runtime SPI usage
- Breaking example changes SHALL fail validation if corresponding examples are not updated

## Capabilities

### New Capabilities
- `examples-constitution`: Constitutional governance for runnable examples across the ego-rs platform. Defines mandatory policy, categories, architecture compliance, testing, documentation, repository structure, and CI governance.

### Modified Capabilities
- `project-constitution`: Add constitutional requirement that examples are mandatory and governed by the Examples Constitution.
- `testing-governance`: Extend coverage and testing requirements to include example code. Examples SHALL compile, run, and be tested with deterministic behavior.
- `architecture-governance`: Extend architecture compliance validation (hexagonal architecture, ports/adapters, dependency direction, CQRS, fail-closed) to apply to all example code exactly as it applies to production code.
- `runtime-abstraction`: Require that runtime SPI examples demonstrate runtime capability ports through constitutional patterns, not through bypass or shortcut approaches.

## Impact

- `openspec/specs/`: New `examples-constitution/` spec directory with `spec.md`
- `openspec/specs/project-constitution/spec.md`: Amendment to add examples governance requirement
- `openspec/specs/testing-governance/spec.md`: Amendment to extend testing requirements to examples
- `openspec/specs/architecture-governance/spec.md`: Amendment to extend architecture compliance to examples
- `openspec/specs/runtime-abstraction/spec.md`: Amendment to require constitutional example patterns for runtime SPI
- `examples/`: New top-level directory with canonical category subdirectories
- Pipeline governance: Example validation gates for compilation, execution, and architecture compliance
- Governance tooling: Architecture validation SHALL span both production and example code
