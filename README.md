# OPSX Engineering Governance

Lightweight engineering governance for OPSX/OpenSpec.

## Overview

OPSX is a lightweight change workflow. This extension adds a quality gate before apply to prevent unreviewed changes.

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

## Commands

### `/opsx:propose`

Propose a new change - create it and generate all artifacts in one step.

```bash
/opsx:propose <change-name>
```

Creates:
- `proposal.md` (what & why)
- `design.md` (how)
- `tasks.md` (implementation steps)
- `metadata.yaml` (approval tracking)

### `/opsx:verify`

Verify proposal quality and approve before apply.

```bash
/opsx:verify <change-name>
```

Evaluates:
- SOLID principles
- Clean architecture
- Hexagonal boundaries
- Maintainability
- Determinism
- Testability
- Dependency inversion
- Mock-only compliance
- Overengineering detection
- Proportional complexity

Outputs: **PASS** | **WARN** | **FAIL**

### `/opsx:apply`

Implement tasks from an OpenSpec change.

```bash
/opsx:apply <change-name>
```

**Fails if not approved:**
```
Change not approved.
Run: /opsx:verify and approve before apply.
```

### `/opsx:archive`

Archive a completed change.

```bash
/opsx:archive <change-name>
```

## Metadata Tracking

Each change has `metadata.yaml`:

```yaml
approved: false
verified: false
```

After `/opsx:verify`:

```yaml
approved: true
verified: true
verdict: pass
issues: []
```

## Engineering Context

See `openspec/engineering-context.md` for:
- Architecture principles
- Quality standards
- Testing strategy
- Verification gate rules

## Design Principles

- **Keep OPSX simple** - No enterprise workflow
- **Extend, don't replace** - Existing lifecycle unchanged
- **Lightweight approval** - metadata.yaml, no workflow engine
- **FAIL blocks apply** - Quality gate enforced
