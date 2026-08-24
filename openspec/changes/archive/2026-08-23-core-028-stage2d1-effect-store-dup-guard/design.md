# Design: CORE-028D1 — Effect Store Duplicate Registration Guard

## Technical Approach

Copy `.adapter()`'s guard shape (`app/mod.rs:305-317`) into `.effect_store()` and
`.effect_retention_store()`, substituting a per-slot `bool` for
`HashSet<TypeId>`. Both methods gain the `pending_error` short-circuit every
sibling already has. `RuntimeBuilder` is untouched: the facade owns all new
state, exactly as it already does for `.adapter()`.

No new abstraction — this guard exists verbatim three times in the file.

## Architecture Decisions

### Decision: presence flags, not `HashSet<TypeId>`

| Option | Tradeoff | Decision |
|--------|----------|----------|
| `bool` per slot | Rejects a second call even with a different concrete type | **Chosen** |
| `HashSet<TypeId>` (adapter shape) | Would let two *different* store types both register | Rejected |

**Rationale**: `RuntimeBuilder` holds one `Option` per slot
(`runtime/builder.rs:472`, `:505-506`), so a second registration of any type is
an overwrite. Adapters are keyed by type and genuinely admit many; these two are
single-slot.

### Decision: asymmetric error payloads

| Variant | Payload | Why |
|---------|---------|-----|
| `DuplicateEffectStore` | `type_name: &'static str` | `effect_store<T>` is generic — `type_name::<T>()` is free, mirrors `DuplicateAdapter` (`error.rs:24`) |
| `DuplicateEffectRetentionStore` | none | `Arc<dyn RetentionMaintenance>` erases the concrete type; adding a generic param would be a signature change (out of scope) |

Both are plain-field variants, not `#[from]` wrappers, because facade-only means
there is no underlying typed error to wrap.

### Decision: `RuntimeBuilder` duplication stays unspecified

**Choice**: resolve the proposal's open question by leaving it.
**Rejected**: spec `with_effect_store` as explicit last-write-wins.
**Rationale**: the facade is now the guarded public composition surface, and the
spec delta belongs to `application-composition`, not `service-sdk`. Documenting
a lower seam adds review surface for zero behavior change.

## Data Flow

    .effect_store(s)
         │
         ├─ pending_error.is_some() ──────────────→ return self (unmodified)
         ├─ effect_store_registered ──→ latch DuplicateEffectStore ──→ return self
         └─ set flag ──→ RuntimeBuilder::with_effect_store ──→ self
                                                                  │
                                          .build() ──→ Err(pending_error) | Ok(App)

## File Changes

| File | Action | Description |
|------|--------|-------------|
| `crates/service-sdk/src/app/error.rs` | Modify | Two new `CompositionError` variants + 2 tests |
| `crates/service-sdk/src/app/mod.rs` | Modify | 2 flags on `AppBuilder`, 2 method bodies, 4 tests |
| `crates/service-sdk/src/runtime/builder.rs` | Unchanged | Zero diff — verify in review |

## Interfaces / Contracts

```rust
// app/error.rs — after DuplicateAdapter, keeping dup variants adjacent
/// A second effect store was registered. Single-slot: rejected even for a
/// different concrete type. `type_name` names the *rejected* registration.
#[error("effect store already registered; second registration of `{type_name}` rejected")]
DuplicateEffectStore { type_name: &'static str },
/// A second effect retention store was registered. No `type_name`: the
/// parameter is `Arc<dyn RetentionMaintenance>`, so no type identity exists.
#[error("effect retention store already registered")]
DuplicateEffectRetentionStore,

// app/mod.rs — AppBuilder, immediately after `adapter_types` (dup-guard state grouped)
effect_store_registered: bool,
effect_retention_store_registered: bool,   // independent slot

// app/mod.rs — body order identical to `.adapter()`
if self.pending_error.is_some() { return self; }              // 1. short-circuit
if self.effect_store_registered {                             // 2. guard + latch
    self.pending_error = Some(CompositionError::DuplicateEffectStore {
        type_name: std::any::type_name::<T>(),
    });
    return self;
}
self.effect_store_registered = true;                          // 3. latch presence
self.runtime_builder = self.runtime_builder.with_effect_store(store);  // 4. delegate
self
```

Step 3 precedes step 4 because delegation is infallible — no rollback path,
unlike `.projection()`'s clone-then-call.

## Testing Strategy

Strict TDD: each test RED before its production line. All unit tests live in the
existing `app/mod.rs` PR6a section (`:1251`), reusing `compat_app()` and
`RecordingEffectStore`.

| Test | Mirrors | Proves |
|------|---------|--------|
| `duplicate_effect_store_registration_is_rejected` | `:819` | `Err(DuplicateEffectStore)`, `type_name == type_name::<RecordingEffectStore>()` |
| `second_effect_store_of_a_different_type_is_still_rejected` | inverts `:838` | `RecordingEffectStore` then `InMemoryEffectStore` still fails; `type_name` names the second — guard is presence-based |
| `duplicate_effect_retention_store_registration_is_rejected` | `:819` | `Err(DuplicateEffectRetentionStore)` |
| `effect_store_and_retention_store_short_circuit_on_a_pending_error` | `.adapter()` short-circuit | a pre-latched `DuplicateAdapter` survives both calls unchanged |
| `duplicate_effect_store_carries_type_name` (error.rs) | `:71` | field round-trip |
| `duplicate_effect_retention_store_display_text` (error.rs) | — | pins the fieldless variant's message |

Independent slots need no new test: `effect_retention_store_composes_with_the_same_instance_via_app_builder`
(`:1542`) already pins `.effect_store(s.clone()).effect_retention_store(s)` and
must stay green unmodified.

Integration/E2E: none — no runtime behavior changes on the happy path.

## Threat Matrix

N/A — in-memory builder state only. No I/O, no routing, no shell, subprocess,
VCS automation, or new trust boundary.

## Migration / Rollout

No migration required. Additive guard; hosts that never double-register are
byte-identical. Revert the single commit to roll back.

## Open Questions

None. The proposal's open question is resolved above.
