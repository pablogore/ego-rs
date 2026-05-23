---
description: Verify proposal quality and approve before apply
---

Verify proposal quality and approve before apply.

**Input**: Optionally specify a change name (e.g., `/opsx-verify add-auth`). If omitted, check if it can be inferred from conversation context. If vague or ambiguous you MUST prompt for available changes.

**Steps**

1. **Select the change**

   If a name is provided, use it. Otherwise:
   - Infer from conversation context if the user mentioned a change
   - Auto-select if only one active change exists
   - If ambiguous, run `openspec list --json` to get available changes and use the **AskUserQuestion tool** to let the user select

   Always announce: "Verifying: <name>" and how to override.

2. **Ensure metadata.yaml exists**

   Check if `openspec/changes/<name>/metadata.yaml` exists.
   
   If not, create it with default values:
   ```yaml
   approved: false
   verified: false
   ```

3. **Run quality checks**

   Evaluate the change against engineering criteria:

   **SOLID Sanity**
   - Single Responsibility: Does each component have one reason to change?
   - Open/Closed: Are components open for extension, closed for modification?
   - Liskov Substitution: Can subclasses replace base classes without breaking?
   - Interface Segregation: Are interfaces client-specific?
   - Dependency Inversion: Are high-level modules independent of low-level details?

   **Clean Architecture**
   - Are business rules independent of frameworks, DB, UI?
   - Do dependencies point inward toward entities?
   - Are there clear boundaries between layers?

   **Hexagonal Boundaries**
   - Are adapters isolated from core domain?
   - Is the domain free of framework dependencies?
   - Are ports/ports clearly defined?

   **Maintainability**
   - Is code readable and self-documenting?
   - Are artifacts complete and consistent?
   - Is there unnecessary complexity?

   **Determinism**
   - Are outputs predictable given same inputs?
   - Is there hidden time/state dependency?
   - Are side effects explicit?

   **Testability**
   - Can units be tested in isolation?
   - Are dependencies injectable?
   - Are tests fast and reliable?

   **Dependency Inversion**
   - Do abstractions drive dependencies?
   - Are concrete implementations swapped via config/injection?

   **Mock-Only Compliance** (unit tests)
   - Are unit tests isolated from external systems?
   - Are mocks used for DB, network, filesystem?

   **Coverage Expectation**
   - Is 95% coverage targeted?
   - Are critical paths covered?

   **Overengineering Detection**
   - Is complexity proportional to problem size?
   - Are patterns justified by requirements?
   - Is there premature abstraction?

   **Proportional Complexity**
   - Does solution match problem scope?
   - Are there unnecessary layers?

4. **Assign verdict**

   **PASS** - All criteria met, no significant issues
   **WARN** - Minor issues that don't block apply
   **FAIL** - Critical issues that block apply

   Document findings in `metadata.yaml`:
   ```yaml
   approved: true
   verified: true
   verdict: pass|warn|fail
   issues:
     - description
   ```

5. **Update metadata.yaml**

   Set `verified: true` and `approved: true` (verify implies approval).
   Record verdict and any issues.

6. **Output results**

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

   Change is ready for /opsx-apply
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
- Verify always sets `verified: true`
- Verify implies approval (sets `approved: true`)
- FAIL blocks apply
- WARN allows apply but documents concerns
- PASS indicates ready for apply
- Keep checks lightweight and practical
- Do not introduce enterprise workflow
