# Specification Quality Checklist: Effect API

**Feature**: [specs/003-effect-api/spec.md](spec.md)

## Checklist

- [ ] Effect as enum with explicit variants (NoEffect, StateMutation, EventEmission, Reply, Composed)
- [ ] Generic type parameters for event, reply, state (not coupled to DomainEvent)
- [ ] Composition via Composed variant (recursive)
- [ ] No runtime types in Effect definitions
- [ ] Effects are value types (Clone, Debug, PartialEq, Eq, Hash)
- [ ] Effect API is independent of ExecutionContext
- [ ] Deterministic testing without infrastructure
- [ ] Exhaustive runtime interpretation
