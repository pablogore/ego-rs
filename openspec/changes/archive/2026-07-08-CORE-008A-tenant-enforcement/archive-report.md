# Archive Report Addendum: CORE-008A — Tenant Enforcement

**Purpose**: Reconcile the archived record with what shipped after archiving.
This addendum does not amend any FR, AD, or architectural decision — it
documents a timeline fact that `tasks.md` and `design.md`, read separately,
do not make explicit.

## Status at Archive (2026-07-08)

`CrossTenantPermit` issuance (`issue_cross_tenant_permit`, TASK-015–019) was
implemented and tested. `TenantResolver::resolve()` did not yet consume an
issued permit — a caller holding a validly-issued cross-tenant permit for a
destination tenant still received `TenantMismatch` on that exact
destination.

During the archive process, TASK-028 was marked complete based on the
implementation status understood at that time. Subsequent verification
performed during PR #143 identified that the authorized cross-tenant
resolution path was not yet fully wired into `TenantResolver::resolve()`.

## Resolution

PR #143 ("Opsx/core 008a fr006 cross tenant grant"), merged 2026-07-09,
completed that integration without changing the architectural design or
functional requirements defined by CORE-008A. Commits `b8fd2bc` (consume
`CrossTenantPermit` in `TenantResolver`) and `ac11ace` (whitespace/clone
fixes from review). `TenantResolver::resolve()` now consumes the grant via
`EstablishedTenantFacts` (`crates/service-sdk/src/runtime/tenant.rs:208`).
The same PR added a new architectural decision, "Fact Establishment vs.
Policy Evaluation," to `design.md`, formalizing the architectural seam this
integration relies on. It was initially numbered AD-013; see "Design.md
Restoration" below for why it is now AD-014.

## Current Status

FR-006 is fully implemented and covered by both positive and negative
tests (`resolve_authorized_cross_tenant_grant_succeeds`,
`issue_cross_tenant_permit_denied_without_capability`, among others in
`crates/service-sdk/tests/` and `tenant.rs`).
`openspec/specs/service-sdk/spec.md` (living spec) reflects the current,
working behavior.

**On `tasks.md` TASK-028**: left unedited as the original implementation
checklist. This addendum is the reconciliation note explaining the
integration timeline.

## Design.md Restoration (GAP-D2)

The initial archived version of `design.md` (created by the archive commit
on 2026-07-08) omitted architectural decisions AD-007 through AD-012, and
the original AD-013 ("Transport-independent Tenant Resolution"), that were
present in the change's design document up to that point. These six
decisions, plus the original AD-013, have been recovered verbatim from the
pre-archive version of `design.md` (`openspec/changes/core-008a-tenant-enforcement/design.md`
at commit `61a752e^`) and restored to this file, in their original form —
title, decision, rationale, tradeoffs, rejected alternatives, consequences,
and FR/Open-Question references unchanged.

Restoring the original AD-013 created a numbering collision with the
"Fact Establishment vs. Policy Evaluation" decision PR #143 added to this
file on 2026-07-09 under the same AD-013 label (see "Resolution" above).
That decision has been renumbered to **AD-014**; its content is otherwise
unchanged. Internal references within `design.md` that pointed to the
PR #143 decision have been updated to AD-014; references to the original,
historical AD-013 are unchanged.

This restoration introduces no new architectural decision, changes no FR,
NFR, or existing decision's content, and does not alter runtime behavior —
it corrects an accidental content loss in the archived record.

**Known, out-of-scope side effect**: several code comments in
`crates/service-sdk/src/{context/mod.rs,runtime/mod.rs,runtime/tenant.rs,runtime/runtime_builder.rs}`
cite "AD-013" referring to the "Fact Establishment vs. Policy Evaluation"
decision, now AD-014. These comments were not updated as part of this
reconciliation (code is out of scope for this addendum) and are left as a
follow-up for whoever next touches those files.
