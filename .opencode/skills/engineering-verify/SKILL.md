---
name: engineering-verify
description: Run engineering quality checks and approve proposals before apply
license: MIT
compatibility: Requires opsx-verify command available.
metadata:
  author: opsx
  version: "1.0"
  generatedBy: "1.3.1"
---

Engineering verification - lightweight quality gate before apply.

**Purpose**

Verify proposal quality using engineering criteria. Output PASS/WARN/FAIL verdict.

Update `metadata.yaml` with verification results.

**Input**

Change name (inferred from context or provided).

**Steps**

1. **Load change context**

   Read `openspec/changes/<name>/.openspec.yaml` for schema.
   Read `openspec/changes/<name>/metadata.yaml` (create if missing).

2. **Evaluate engineering criteria**

   **SOLID**
   - Single Responsibility: One reason to change per component
   - Open/Closed: Open for extension, closed for modification
   - Liskov Substitution: Subclasses replace base classes safely
   - Interface Segregation: Client-specific interfaces
   - Dependency Inversion: Depend on abstractions

   **Clean Architecture**
   - Business rules independent of frameworks/DB/UI
   - Dependencies point inward to entities
   - Clear layer boundaries

   **Hexagonal Boundaries**
   - Adapters isolated from domain
   - Domain free of framework dependencies
   - Ports clearly defined

   **Maintainability**
   - Readable, self-documenting code
   - Complete, consistent artifacts
   - No unnecessary complexity

   **Determinism**
   - Predictable outputs for same inputs
   - No hidden time/state dependency
   - Explicit side effects

   **Testability**
   - Units testable in isolation
   - Injectable dependencies
   - Fast, reliable tests

   **Dependency Inversion**
   - Abstractions drive dependencies
   - Concrete implementations swappable

   **Mock-Only Compliance**
   - Unit tests isolated from external systems
   - Mocks for DB, network, filesystem

   **Coverage Target**
   - 95% coverage targeted
   - Critical paths covered

   **Overengineering Detection**
   - Complexity proportional to problem
   - Patterns justified by requirements
   - No premature abstraction

   **Proportional Complexity**
   - Solution matches problem scope
   - No unnecessary layers

3. **Determine verdict**

   **PASS**: All criteria met
   **WARN**: Minor issues, no blockers
   **FAIL**: Critical issues, blocks apply

4. **Update metadata.yaml**

   ```yaml
   approved: true
   verified: true
   verdict: pass|warn|fail
   issues:
     - description (if any)
   ```

5. **Output results**

   ```
   ## Verification Results: <change-name>

   **Verdict:** PASS | WARN | FAIL

   **Criteria:**
   - SOLID: ✓
   - Clean Architecture: ✓
   - Hexagonal Boundaries: ✓
   - Maintainability: ✓
   - Determinism: ✓
   - Testability: ✓
   - Dependency Inversion: ✓
   - Mock-Only Compliance: ✓
   - Overengineering: ✓
   - Proportional Complexity: ✓

   **Issues:**
   (list if any)

   **Status:** approved: true, verified: true
   ```

**Output On FAIL**

```
## Verification Results: <change-name>

**Verdict:** FAIL

**Critical Issues:**
- Issue 1
- Issue 2

**Status:** approved: false, verified: true

Change not approved.

Fix the issues above and run /opsx-verify again.
```

**Guardrails**
- Keep checks lightweight
- Do not introduce enterprise workflow
- Verify implies approval
- FAIL blocks apply
- WARN documents concerns but allows apply
- Be practical, not bureaucratic
