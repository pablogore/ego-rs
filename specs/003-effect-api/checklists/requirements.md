# Specification Quality Checklist: Effect API

**Feature**: [specs/003-effect-api/spec.md](spec.md)

## Checklist

- [x] Effect as enum with explicit variants (NoEffect, StateMutation, EventEmission, Reply, Composed)
- [x] Generic type parameters for event, reply, state (not coupled to DomainEvent)
- [x] Composition via Composed variant (recursive)
- [x] No runtime types in Effect definitions
- [ ] Effects are value types (Clone, Debug, PartialEq, Eq, Hash)
- [x] Effect API is independent of ExecutionContext
- [x] Deterministic testing without infrastructure
- [ ] Exhaustive runtime interpretation
