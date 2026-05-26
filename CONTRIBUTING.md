# Contributor Checklist

Before submitting any future OpenSpec change, verify SPEC-000 compliance:

- [ ] The change starts from an OpenSpec proposal before implementation.
- [ ] Implementation tasks are traceable to proposal, design, and spec artifacts.
- [ ] Domain and application behavior remains deterministic by default.
- [ ] Validation, authorization, parsing, and governance decisions fail closed.
- [ ] State changes are represented through explicit inputs, outputs, events, or ports.
- [ ] Specs, decisions, events, and migrations preserve append-only lineage.
- [ ] Architecture work complies with `architecture-governance`.
- [ ] Testable code complies with `testing-governance`, including mock-first tests and minimum coverage.
- [ ] New production workflows include structured observability.
- [ ] Breaking changes document compatibility, migration, and rollback impact.
- [ ] Constitution changes are proposed as dedicated OpenSpec amendments.
