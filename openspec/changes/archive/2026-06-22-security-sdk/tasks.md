# Tasks: security-sdk

[Full tasks document preserved from openspec/changes/security-sdk/tasks.md — 35 tasks across 13 phases, covering workspace scaffolding through final verification. See the original file for complete task specifications, acceptance criteria, and workload forecasts.]

## Summary

- **Total tasks**: 35 organized across 13 phases
- **Estimated changed lines**: ~2,360
- **Delivery strategy**: Stacked PRs (3 PRs to main)
- **PR-1 (Core Models)**: ~735 lines — pure value types, SecurityContext, workspace setup
- **PR-2 (Providers)**: ~1,190 lines — authentication + authorization contracts + providers
- **PR-3 (ServiceContext Integration)**: ~440 lines — service-sdk changes + 5 integration tests
- **Budget risk**: PR-1 and PR-2 exceed 400 lines; size:exception required
- **All tasks completed**: [x] 92/92 checks marked complete
