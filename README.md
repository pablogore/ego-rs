# OPSX/OpenSpec Proposal Mode

Generate specification proposals only. No implementation.

## Project Constitution (SPEC-000)

All project changes must comply with [SPEC-000: Project Constitution](openspec/changes/spec-000-project-constitution-objetivo/specs/project-constitution/spec.md). This constitution defines immutable rules for deterministic-first behavior, fail-closed decisions, explicit state, append-only lineage, OpenSpec-driven development, mandatory hexagonal architecture, CQRS + event-driven design, >=95% test coverage, no real resources in unit tests, observability by default, and backward compatibility.

Future OpenSpec changes must reference SPEC-000 and demonstrate constitution compliance. Amendments to the constitution require a dedicated OpenSpec change that preserves the rationale for the previous rule.

## Quick Start

```bash
# Create a proposal
"create endpoint returning uuid and timestamp"
```

## What It Does

Transforms implementation wording into capability proposals:

- `create endpoint` → `expose capability through transport`
- `uuid` → `identifier`
- `timestamp` → `temporal metadata`

## Output Files

### proposal.md

- Why
- Impact
- Risks
- NFRs

### design.md

- Purpose
- Rules
- Anti-patterns
- Review checklist

### spec.md

- Capability requirements
- Scenarios
- Expected behavior
- Constraints

### tasks.md

- Capability tasks
- Validation tasks
- Testing tasks

## Rules

1. **Preserve intent** - Don't generalize domain
2. **One abstraction level** - uuid → identifier, not → entity reference
3. **Proportional scope** - Small request → small proposal
4. **Verb-preserving** - "create capability" not "creation capability"
5. **No speculative architecture** - Don't invent CRUD, lifecycle, validation

## Examples

### Input

```
create endpoint returning uuid and timestamp
```

### Output

Capability proposal returning identifier and temporal metadata.

### Input

```
create hello endpoint
```

### Output

Expose hello capability through transport.

## Don't Do

- ❌ Ask clarification questions
- ❌ Expand into CRUD workflows
- ❌ Add validation rules unless requested
- ❌ Invent entities or business semantics
- ❌ Generate implementation

## Success Criteria

Result feels like:

- ✓ Proportional
- ✓ Capability-driven
- ✓ Semantically close to user intent
- ✓ Specification-oriented
- ✓ Boring engineering
