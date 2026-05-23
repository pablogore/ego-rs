---
name: engineering-context
description: Project engineering context - architecture, testing, quality standards
license: MIT
compatibility: Built-in context reference.
metadata:
  author: opsx
  version: "1.0"
  generatedBy: "1.3.1"
---

Engineering Context Reference

## Architecture

- **Hexagonal Architecture**: Domain isolated from adapters
- **Clean Architecture**: Business rules independent of frameworks/DB/UI
- **SOLID**: Single Responsibility, Open/Closed, Liskov, Interface Segregation, Dependency Inversion

## Quality Standards

- **Maintainability First**: Readable, self-documenting code
- **Deterministic Behavior**: No hidden time/state dependencies
- **95% Coverage Target**: Critical paths covered
- **Mock-Only Unit Tests**: Isolated, fast, reliable
- **Testcontainers**: Real infra validation for integration tests

## Complexity Rules

- **Proportional Complexity**: Solution matches problem scope
- **No Overengineering**: Patterns justified by requirements
- **No Premature Abstraction**: Solve actual problems

## Testing Strategy

**Unit Tests**
- Mock-only dependencies
- No DB, network, filesystem, external systems
- No hidden time dependency
- Deterministic, isolated, fast, readable

**Integration Tests**
- Optional
- Use Testcontainers for real infra validation
- No examples, no code snippets

## Verification Gate

Before apply:
1. Run `/opsx-verify`
2. Check verdict: PASS/WARN/FAIL
3. FAIL blocks apply
4. WARN documents concerns
5. PASS allows apply

## Metadata Tracking

Each change has `metadata.yaml`:
```yaml
approved: false
verified: false
verdict: pass|warn|fail
issues: []
```

After `/opsx-verify`:
```yaml
approved: true
verified: true
verdict: pass
```

## Workflow

```
/opsx:propose
    ↓
draft (metadata: approved=false, verified=false)
    ↓
/opsx:continue (optional)
    ↓
/opsx:verify
    ↓
approved (metadata: approved=true, verified=true)
    ↓
/opsx:apply
    ↓
/opsx:archive
```

## Rule

`/opsx:apply` MUST fail if `metadata.approved != true`.

Error message:
```
Change not approved.
Run: /opsx:verify and approve before apply.
```
