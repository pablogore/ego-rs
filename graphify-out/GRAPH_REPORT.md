# Graph Report - ego-rs  (2026-05-25)

## Corpus Check
- 47 files · ~17,616 words
- Verdict: corpus is large enough that graph structure adds value.

## Summary
- 414 nodes · 375 edges · 44 communities (35 shown, 9 thin omitted)
- Extraction: 100% EXTRACTED · 0% INFERRED · 0% AMBIGUOUS
- Token cost: 0 input · 0 output

## Graph Freshness
- Built from commit: `fbe29bd6`
- Run `git rev-parse HEAD` and compare to check if the graph is stale.
- Run `graphify update .` after code changes (no API cost).

## Community Hubs (Navigation)
- [[_COMMUNITY_Community 0|Community 0]]
- [[_COMMUNITY_Community 1|Community 1]]
- [[_COMMUNITY_Community 2|Community 2]]
- [[_COMMUNITY_Community 3|Community 3]]
- [[_COMMUNITY_Community 4|Community 4]]
- [[_COMMUNITY_Community 5|Community 5]]
- [[_COMMUNITY_Community 6|Community 6]]
- [[_COMMUNITY_Community 7|Community 7]]
- [[_COMMUNITY_Community 8|Community 8]]
- [[_COMMUNITY_Community 9|Community 9]]
- [[_COMMUNITY_Community 10|Community 10]]
- [[_COMMUNITY_Community 11|Community 11]]
- [[_COMMUNITY_Community 12|Community 12]]
- [[_COMMUNITY_Community 13|Community 13]]
- [[_COMMUNITY_Community 14|Community 14]]
- [[_COMMUNITY_Community 15|Community 15]]
- [[_COMMUNITY_Community 16|Community 16]]
- [[_COMMUNITY_Community 17|Community 17]]
- [[_COMMUNITY_Community 18|Community 18]]
- [[_COMMUNITY_Community 19|Community 19]]
- [[_COMMUNITY_Community 20|Community 20]]
- [[_COMMUNITY_Community 21|Community 21]]
- [[_COMMUNITY_Community 22|Community 22]]
- [[_COMMUNITY_Community 24|Community 24]]
- [[_COMMUNITY_Community 25|Community 25]]
- [[_COMMUNITY_Community 26|Community 26]]
- [[_COMMUNITY_Community 27|Community 27]]
- [[_COMMUNITY_Community 28|Community 28]]
- [[_COMMUNITY_Community 29|Community 29]]
- [[_COMMUNITY_Community 30|Community 30]]
- [[_COMMUNITY_Community 31|Community 31]]
- [[_COMMUNITY_Community 32|Community 32]]
- [[_COMMUNITY_Community 33|Community 33]]
- [[_COMMUNITY_Community 34|Community 34]]
- [[_COMMUNITY_Community 35|Community 35]]
- [[_COMMUNITY_Community 36|Community 36]]
- [[_COMMUNITY_Community 38|Community 38]]

## God Nodes (most connected - your core abstractions)
1. `ADDED Requirements` - 12 edges
2. `ADDED Requirements` - 11 edges
3. `Design: Canonical Contract Governance` - 10 edges
4. `OPSX/OpenSpec Proposal Mode` - 9 edges
5. `Tasks: Foundation 002 — Canonical Contract Governance` - 9 edges
6. `Foundation 002: Canonical Contract Governance` - 8 edges
7. `Phase 4 - Per-Task Implementation Guard` - 7 edges
8. `ADDED Requirements` - 7 edges
9. `ADDED Requirements` - 7 edges
10. `ADDED Requirements` - 6 edges

## Surprising Connections (you probably didn't know these)
- None detected - all connections are within the same source files.

## Communities (44 total, 9 thin omitted)

### Community 0 - "Community 0"
Cohesion: 0.06
Nodes (31): ADDED Requirements, Requirement: Append-only lineage, Requirement: Backward compatibility strategy, Requirement: Constitution amendments, Requirement: Deterministic-first behavior, Requirement: Explicit state only, Requirement: Fail-closed decisions, Requirement: Mandatory architecture governance (+23 more)

### Community 1 - "Community 1"
Cohesion: 0.06
Nodes (31): ADDED Requirements, Requirement: Backward compatibility rules, Requirement: Buf lint and breaking checks, Requirement: Canonical contracts repository structure, Requirement: Contract-first development, Requirement: Contract ownership and review, Requirement: Contract testing governance, Requirement: CQRS contract taxonomy (+23 more)

### Community 2 - "Community 2"
Cohesion: 0.07
Nodes (25): ALLOWED_TOOLS, allowedPaths, BLOCKED_PATTERNS, command, DISCOVERY_COMMANDS, id, idx, last (+17 more)

### Community 3 - "Community 3"
Cohesion: 0.08
Nodes (25): code:text (FAIL_CLOSED: real_tool_unavailable), code:text (FAIL_CLOSED:), code:text (FAIL_CLOSED:), code:text (FAIL_CLOSED:), code:text (Using change: <name>), code:text (Using change: <name>), code:text (Using change: <name>), code:text (/opsx-apply foundation-001-workspace-monorepo-structure 3.2) (+17 more)

### Community 4 - "Community 4"
Cohesion: 0.10
Nodes (20): code:bash (# Create a proposal), code:block2 (create endpoint returning uuid and timestamp), code:block3 (create hello endpoint), design.md, Don't Do, Examples, Input, Input (+12 more)

### Community 5 - "Community 5"
Cohesion: 0.10
Nodes (20): ADDED Requirements, Requirement: Canonical workspace layout, Requirement: Crate naming convention, Requirement: Dependency rules mirror architecture governance, Requirement: Layer ownership source of truth, Requirement: Package groups are not layers, Requirement: Public module boundaries, Scenario: Cross-crate import is reviewed (+12 more)

### Community 6 - "Community 6"
Cohesion: 0.11
Nodes (17): Check for context, code:block1 (┌─────────────────────────────────────────┐), code:bash (openspec list --json), code:block3 (User: I'm thinking about adding real-time collaboration), code:block4 (User: The auth system is a mess), code:block5 (User: /opsx-explore add-auth-system), code:block6 (User: Should we use Postgres or SQLite?), code:block7 (## What We Figured Out) (+9 more)

### Community 7 - "Community 7"
Cohesion: 0.11
Nodes (16): ADDED Requirements, Requirement: 95% minimum test coverage, Requirement: Mock-first testing strategy, Requirement: No real resources in test suite, Requirement: Test file co-location, Requirement: Trait-based mocking infrastructure, Scenario: cargo test runs offline, Scenario: CI passes at or above 95% (+8 more)

### Community 8 - "Community 8"
Cohesion: 0.13
Nodes (13): ADDED Requirements, Requirement: Hexagonal architecture layers, Requirement: No cross-layer imports in application code, Requirement: Ports and adapters pattern, Requirement: SOLID principles compliance, Scenario: Domain has no internal project dependencies, Scenario: Event store accessed through trait, Scenario: Handler imports only domain and application traits (+5 more)

### Community 9 - "Community 9"
Cohesion: 0.13
Nodes (14): compaction, auto, tail_turns, name, npm, options, model, apiKey (+6 more)

### Community 10 - "Community 10"
Cohesion: 0.14
Nodes (13): Artifact Rules, Change Selection, code:text (What change do you want to work on? Describe what you want t), code:bash (openspec new change "<name>"), code:bash (openspec status --change "<name>" --json), code:text (Using change: <name>), code:text (Using change: <name>), code:text (Using change: <name>) (+5 more)

### Community 11 - "Community 11"
Cohesion: 0.17
Nodes (11): Check for context, code:block1 (┌─────────────────────────────────────────┐), code:bash (openspec list --json), Ending Discovery, Guardrails, OpenSpec Awareness, The Stance, What You Don't Have To Do (+3 more)

### Community 12 - "Community 12"
Cohesion: 0.17
Nodes (11): Adoption Path, Buf Governance, code:block1 (contracts/), CQRS Taxonomy, Design: Canonical Contract Governance, Generation Policy, Ownership and Review, Principles (+3 more)

### Community 13 - "Community 13"
Cohesion: 0.23
Nodes (12): context, output, models, qwen-architect, qwen-fast, qwen-heavy, limit, name (+4 more)

### Community 14 - "Community 14"
Cohesion: 0.20
Nodes (9): Canonical Workspace Groups, Context, Crate Naming, Decisions, Dependency Rule Alignment, Goals / Non-Goals, Layer Ownership, Module Boundaries (+1 more)

### Community 15 - "Community 15"
Cohesion: 0.20
Nodes (9): 1. Repository structure, 2. Buf governance, 3. Generation policy, 4. CQRS taxonomy, 5. Backward compatibility, 6. Ownership and review, 8. Buf lifecycle integration, 9. Testing governance (+1 more)

### Community 16 - "Community 16"
Cohesion: 0.22
Nodes (8): Cobertura 95% global, no por crate, Context, Decisions, Dos specs separadas: arquitectura y testing, Goals / Non-Goals, Gobierno como specs, no como documento aparte, Reglas expresadas como SHALL/MUST (normativas, no sugerencias), Risks / Trade-offs

### Community 17 - "Community 17"
Cohesion: 0.22
Nodes (8): Amendments Require OpenSpec Changes, Constitution as a Dedicated Capability, Context, Decisions, Existing Governance Specs Remain Enforcement Standards, Goals / Non-Goals, No Waivers, Risks / Trade-offs

### Community 18 - "Community 18"
Cohesion: 0.22
Nodes (8): Capabilities, Foundation 002: Canonical Contract Governance, Goal, Impact, Out of Scope, Problem, Risks, Scope

### Community 19 - "Community 19"
Cohesion: 0.29
Nodes (6): Capabilities, Impact, Modified Capabilities, New Capabilities, What Changes, Why

### Community 20 - "Community 20"
Cohesion: 0.29
Nodes (6): code:bash (mkdir -p openspec/changes/archive), code:bash (mv openspec/changes/<name> openspec/changes/archive/YYYY-MM-), code:block3 (## Archive Complete), code:block4 (## Archive Complete), code:block5 (## Archive Complete (with warnings)), code:block6 (## Archive Failed)

### Community 21 - "Community 21"
Cohesion: 0.29
Nodes (6): Capabilities, Impact, Modified Capabilities, New Capabilities, What Changes, Why

### Community 22 - "Community 22"
Cohesion: 0.29
Nodes (6): Capabilities, Impact, Modified Capabilities, New Capabilities, What Changes, Why

### Community 24 - "Community 24"
Cohesion: 0.40
Nodes (4): 1. Workspace Layout, 2. Layer Ownership, 3. Naming And Boundaries, 4. Governance Alignment

### Community 25 - "Community 25"
Cohesion: 0.40
Nodes (4): 1. Governance Specs Validation, 2. CI Enforcement Setup, 3. Developer Tooling, 4. Documentation

### Community 27 - "Community 27"
Cohesion: 0.50
Nodes (3): code:text (Using change: <name>), code:text (Using change: <name>), code:text (/opsx-apply project-governance)

### Community 28 - "Community 28"
Cohesion: 0.50
Nodes (3): code:bash (mkdir -p openspec/changes/archive), code:bash (mv openspec/changes/<name> openspec/changes/archive/YYYY-MM-), code:block3 (## Archive Complete)

### Community 29 - "Community 29"
Cohesion: 0.50
Nodes (3): 1. Constitution Spec, 2. Contributor Documentation, 3. Governance Enforcement

## Knowledge Gaps
- **265 isolated node(s):** `CommandHandler`, `QueryHandler`, `Command`, `Query`, `HelloQuery` (+260 more)
  These have ≤1 connection - possible missing edges or undocumented components.
- **9 thin communities (<3 nodes) omitted from report** — run `graphify query` to explore isolated nodes.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `litellm` connect `Community 9` to `Community 13`?**
  _High betweenness centrality (0.003) - this node is a cross-community bridge._
- **What connects `CommandHandler`, `QueryHandler`, `Command` to the rest of the system?**
  _265 weakly-connected nodes found - possible documentation gaps or missing edges._
- **Should `Community 0` be split into smaller, more focused modules?**
  _Cohesion score 0.06060606060606061 - nodes in this community are weakly interconnected._
- **Should `Community 1` be split into smaller, more focused modules?**
  _Cohesion score 0.0625 - nodes in this community are weakly interconnected._
- **Should `Community 2` be split into smaller, more focused modules?**
  _Cohesion score 0.06666666666666667 - nodes in this community are weakly interconnected._
- **Should `Community 3` be split into smaller, more focused modules?**
  _Cohesion score 0.07692307692307693 - nodes in this community are weakly interconnected._
- **Should `Community 4` be split into smaller, more focused modules?**
  _Cohesion score 0.09523809523809523 - nodes in this community are weakly interconnected._