# Interpretation Error Contract

**Status**: Draft | **Spec**: [spec.md](../spec.md)

## Purpose

Canonical runtime-owned error type for effect interpretation failures. Defined in the runtime layer, not in `ego-domain`.

## Contract

```rust
/// Errors that occur during effect interpretation.
///
/// Owned by the runtime layer. Runtimes MAY extend with additional
/// variants for runtime-specific concerns.
pub enum EffectInterpretationError {
    /// Runtime does not support a specific effect variant.
    UnsupportedEffect,
    /// Composition violates runtime rules.
    InvalidComposition,
    /// Mutually incompatible effects.
    ConflictingEffects,
}
```

## Implementer Requirements

1. **Every runtime** MUST:
   - Evaluate every `Effect` variant explicitly
   - Return `UnsupportedEffect` for variants it does not implement
   - Return `InvalidComposition` for compositions that violate runtime rules
   - Return `ConflictingEffects` for incompatible effect combinations
   - NOT silently ignore any variant

2. **Runtimes** MAY:
   - Add additional error variants beyond the canonical set
   - Include error context (messages, source locations) in custom variants
