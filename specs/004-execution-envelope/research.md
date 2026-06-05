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
Envelope carries payload as a type parameter `P`. The envelope struct is `ExecutionEnvelope<P>`. Payload is **mandatory** — `payload: P`, never `Option<P>`.

### Rationale
- Payload type varies per execution model (command, event, workflow message, etc.)
- Generic parameter preserves type safety without boxing
- Runtime crates monomorphize for their specific payload types
- Domain types stay pure — no serde::Value or Box<dyn Any> needed
- **Payload-less execution models use `ExecutionEnvelope<()>`**: `()` is Rust's idiomatic zero-sized type for "no data." This avoids Option branching on every payload access while still supporting signal-only use cases (saga triggers, projection rebuild signals, etc.)
- `Option<P>` was rejected: it forces every consumer to match/unwrap, weakens the contract that an envelope always carries a payload (FR-002), and adds serialization complexity for the `None` variant

### Alternatives Considered
| Alternative | Rejected Because |
|-------------|-----------------|
| Box<dyn Any> | Loses type safety; forces downcasting in every handler |
| serde_json::Value | Forces all payloads through JSON; couples envelope to serialization |

## Decision 3: Context Construction Ownership

### Decision
`DomainExecutionContext` (domain-owned concrete type) implements `From<ExecutionEnvelope<P>>`. Runtime implementations use named constructors (e.g. `RuntimeExecutionContext::from_envelope()`).

### Rationale
- `ExecutionContext` is a trait — it cannot directly implement `From`
- `DomainExecutionContext` lives in `ego-domain` alongside `ExecutionEnvelope` — no runtime deps required
- `From` trait is idiomatic Rust for infallible conversions
- Runtime implementations are free to add their own envelope-to-context conversion as a named method (e.g. `from_envelope()`)
- Handlers receive `&dyn ExecutionContext`, so the conversion happens before handler dispatch

### Alternatives Considered
| Alternative | Rejected Because |
|-------------|-----------------|
| `From<ExecutionEnvelope<P>>` for `ExecutionContext` (trait) | Impossible — traits cannot implement `From` |
| `RuntimeExecutionContext` implements `From<ExecutionEnvelope<P>>` | Would force runtime `From` dependency; named `from_envelope()` is already implemented and preferred |
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

## Decision 5: Serialization Strategy

### Decision
ExecutionEnvelope derives serde `Serialize + Deserialize` traits. The transport layer selects the specific wire format.

### Rationale
- serde is Rust's standard serialization framework — it defines traits (`Serialize`, `Deserialize`), not a wire format
- Deriving serde traits makes the envelope serializable by any serde-compatible format (JSON, MessagePack, protobuf via serde, etc.)
- The transport layer still owns the format decision — HTTP may use JSON, gRPC may use protobuf
- User Story 3 requires round-trip testing, which needs serde derives
- Without derives, every runtime would need its own serialization wrapper — violates "avoid duplicate modules"
- serde is ubiquitous in the Rust ecosystem; adding it as a dependency to `ego-domain` is standard practice

### Alternatives Considered
| Alternative | Rejected Because |
|-------------|-----------------|
| No serde derives | Blocks US-3 round-trip testing; forces each runtime to duplicate serialization logic |
| Enforce a specific format (e.g., JSON) | Breaks gRPC and binary protocol use cases; couples envelope to format |

## Decision 6: Runtime Type — RuntimeExecutionContext

### Decision
The runtime crate provides `RuntimeExecutionContext` with a named `from_envelope()` constructor. This exists alongside the domain's `DomainExecutionContext` which uses the `From` trait.

Both types implement the `ExecutionContext` trait but serve different roles:
- `DomainExecutionContext`: domain-owned, infallible conversion from envelope via `From`, used when no runtime-specific behavior is needed
- `RuntimeExecutionContext`: runtime-owned, named constructor `from_envelope()`, may carry runtime-specific lifecycle or observability concerns

### Rationale
- "Patch over rewrite" — the existing struct has working tests and functionality
- Adding an envelope-based constructor is additive, not breaking
- Two concrete implementations of one trait is idiomatic Rust; no duplication of conversion logic
- Follows the same approach as 002 (refactoring existing struct)
