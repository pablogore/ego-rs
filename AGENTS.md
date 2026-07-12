# ego-rs — Agent Skills Index

When working on this project, load the relevant skill(s) BEFORE writing any code.

Naming convention: `ego-rs-*` skills are repo-specific workflow skills. Unprefixed skills are portable writing or work-unit skills and intentionally keep their canonical names. `sdd-*` skills drive the Spec-Driven Development cycle.

## Project Context

- **Product**: [`PRD.md`](PRD.md) — vision, principles, architecture, roadmap
- **Engineering architecture**: [`ARCHITECTURE.md`](ARCHITECTURE.md)
- **Canonical specs**: [`openspec/specs/`](openspec/specs/)

## How to Use

1. Check the trigger column to find skills that match your current task
2. Load the skill by reading the SKILL.md file at the listed path
3. Follow ALL patterns and rules from the loaded skill
4. Multiple skills can apply simultaneously

## Skills

| Skill | Trigger | Path |
|-------|---------|------|
| `ego-rs-security` | When writing or reviewing queries, auth, JWT, tenant isolation, or any user-input boundary. | [`skills/security/SKILL.md`](skills/security/SKILL.md) |
| `ego-rs-testing` | When writing tests, mocks, or deciding where a test belongs (unit vs. integration). | [`skills/testing/SKILL.md`](skills/testing/SKILL.md) |
| `ego-rs-issue-creation` | When creating a GitHub issue, reporting a bug, or requesting a feature. | [`skills/issue-creation/SKILL.md`](skills/issue-creation/SKILL.md) |
| `ego-rs-branch-pr` | When creating a pull request, opening a PR, or preparing changes for review. | [`skills/branch-pr/SKILL.md`](skills/branch-pr/SKILL.md) |
| `ego-rs-chained-pr` | When a change is too large for one review, or when creating chained/stacked pull requests. | [`skills/chained-pr/SKILL.md`](skills/chained-pr/SKILL.md) |
| `cognitive-doc-design` | When writing docs that must reduce cognitive load for readers or reviewers. | [`skills/cognitive-doc-design/SKILL.md`](skills/cognitive-doc-design/SKILL.md) |
| `comment-writer` | When drafting human comments, PR feedback, issue replies, or async updates. | [`skills/comment-writer/SKILL.md`](skills/comment-writer/SKILL.md) |
| `work-unit-commits` | When splitting implementation work into deliverable commits or chained PRs. | [`skills/work-unit-commits/SKILL.md`](skills/work-unit-commits/SKILL.md) |
| `sdd-explore` | When exploring an idea or feature before committing to a change. | `~/.claude/skills/sdd-explore/SKILL.md` |
| `sdd-init` | When initializing SDD context for the first time in a session. | `~/.claude/skills/sdd-init/SKILL.md` |
| `sdd-apply` | When implementing SDD tasks — writes code following specs and design. | `~/.claude/skills/sdd-apply/SKILL.md` |
| `sdd-verify` | When validating that implementation matches specs, design, and tasks. | `~/.claude/skills/sdd-verify/SKILL.md` |
| `sdd-archive` | When closing a completed and verified SDD change. | `~/.claude/skills/sdd-archive/SKILL.md` |
| `judgment-day` | When running blind dual review or adversarial review before merging. | `~/.claude/skills/judgment-day/SKILL.md` |
| `skill-creator` | When creating new skills or documenting AI usage patterns. | `~/.claude/skills/skill-creator/SKILL.md` |
