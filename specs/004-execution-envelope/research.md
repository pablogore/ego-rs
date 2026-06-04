# Research: Execution Envelope Design Decisions

**Date**: 2026-06-04 | **Spec**: [spec.md](spec.md) | **Plan**: [plan.md](plan.md)

## Decision 1: Struct over Trait

### Decision
Define `ExecutionEnvelope<P>` as a generic struct (not a trait).

### Rationale
- The envelope is a data carrier — it has no behavior beyond carrying fields
- A trait would require `dyn` dispatch or monomorphization for no behavioral benefit
- A struct is serializable, clonable, and easy to construct in tests
- Runtime-specific envelope extensions can wrap or embed this struct without implementing a trait

### Alternatives Considered
| Alternative | Rejected Because |
|-------------|-----------------|
| Trait-based | Adds abstraction without behavioral value — envelope is pure data |
| Un-typed (serde_json::Value) payload | Loses type safety for payload; forces all consumers to handle runtime deserialization |

## Decision 2: Payload as Generic Parameter

### Decision
Envelope carries payload as a type parameter `P`. The envelope struct is `ExecutionEnvelope<P>`.

### Rationale
- Payload type varies per execution model (command, event, workflow message, etc.)
- Generic parameter preserves type safety without boxing
- Runtime crates monomorphize for their specific payload types
- Domain types stay pure — no serde::Value or Box<dyn Any> needed

### Alternatives Considered
| Alternative | Rejected Because |
|-------------|-----------------|
| Box<dyn Any> | Loses type safety; forces downcasting in every handler |
| serde_json::Value | Forces all payloads through JSON; couples envelope to serialization |

## Decision 3: Context Construction via From Trait

### Decision
Implement `From<ExecutionEnvelope<P>>` for the runtime's ExecutionContext struct (or a `ExecutionContext::from_envelope` constructor).

### Rationale
- `From` trait is idiomatic Rust for infallible conversions
- The envelope has all fields needed to construct context — no fallible conversion required
- Handlers receive `&dyn ExecutionContext`, so the conversion happens at the runtime boundary

### Alternatives Considered
| Alternative | Rejected Because |
|-------------|-----------------|
| Envelope implements ExecutionContext directly | Envelope carries payload — context should not expose payload; violates single responsibility |
| Manual field copying | More boilerplate than From trait; no safety benefit |

## Decision 4: Crate Ownership

### Decision
Define `ExecutionEnvelope` in `ego-domain` alongside `ExecutionContext` (002 types).

### Rationale
- Follows the established pattern: domain owns contracts and carriers
- Envelope reuses identity/correlation types already in `ego-domain`
- Runtime crates import both ExecutionEnvelope and ExecutionContext from the same crate
- No circular dependency risk — domain has no runtime dependencies

### Alternatives Considered
| Alternative | Rejected Because |
|-------------|-----------------|
| `ego-runtime` | Runtime would own the carrier contract, violating domain ownership pattern |
| New crate | Unnecessary — contradicts "avoid duplicate modules" rule |

## Decision 5: Serialization Independence

### Decision
ExecutionEnvelope does not define a serialization format. Transport layers convert between wire format and ExecutionEnvelope.

### Rationale
- HTTP may use JSON, gRPC may use protobuf, in-process uses direct construction
- A serialization constraint would couple the envelope to a specific transport
- Transport adapters already exist in `crates/transport` — they serialize/deserialize at the boundary

### Alternatives Considered
| Alternative | Rejected Because |
|-------------|-----------------|
| Derive Serialize/Deserialize on envelope (always) | OK to derive, but not required — transport may need custom serialization |
| Enforce JSON | Breaks gRPC and binary protocol use cases |

## Decision 6: Reuse Existing Runtime Struct

### Decision
The existing `crates/runtime/src/context.rs` CommandContext struct is refactored to accept `ExecutionEnvelope` for construction and implement the domain `ExecutionContext` trait.

### Rationale
- "Patch over rewrite" — the existing struct has working tests and functionality
- Adding an envelope-based constructor is additive, not breaking
- The struct gains identity, correlation, and metadata fields from the envelope
- Follows the same approach as 002 Decision 4 (refactoring existing struct)
