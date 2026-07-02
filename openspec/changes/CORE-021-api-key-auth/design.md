# Design: API Key Authentication (CORE-021)

## Technical Approach

New self-contained crate `security-apikey` implementing the SDK's synchronous
`AuthenticationProvider` for opaque API keys. Structure mirrors `security-jwt`
(value objects + resolver SPI + thin provider), but the resolver is
**synchronous** — API key lookup is a local map read, so no async-to-sync
thread-pool bridge is needed (unlike JWT's async `KeyResolver`). The provider
owns the strict 4-step validation flow from the proposal and the constant-time
comparison; the resolver owns only cache-first lookup and hash construction.

## Architecture Decisions

### AD-1: Resolver is synchronous

| Option | Tradeoff | Decision |
|--------|----------|----------|
| Async `lookup` + `futures_executor` bridge (JWT pattern) | 60+ lines of thread-pool plumbing, matches JWT | rejected |
| Sync `lookup` (Basic `CredentialVerifier` pattern) | No I/O in `authenticate` is already the AD-004 contract | **chosen** |

Rationale: `authenticate` is sync and AD-004 forbids I/O inside it. A sync
`lookup` returning from a warmed cache is the honest contract. JWT only went
async because JWKS refresh returns futures; API keys have no such upstream.
Mirrors `CredentialVerifier::verify` which is already sync in this codebase.

### AD-2: `ApiKeyHash` internal representation

| Option | Tradeoff | Decision |
|--------|----------|----------|
| PHC string via `password-hash` | Pulls a parser + trait ecosystem; algorithm-agnostic | rejected for reference |
| Raw `Vec<u8>` + algorithm tag (private) | Minimal deps, fully opaque public API | **chosen** |

`ApiKeyHash` is completely opaque externally — no fields, no `HashAlgorithm`
enum in the public API. Internally stores `(algorithm_tag, digest: Vec<u8>)`;
`verify(secret)` re-hashes under the tagged algorithm and compares with
`subtle::ConstantTimeEq`. The algorithm tag is a private implementation detail.
Named constructors (`ApiKeyHash::sha256(digest)`) are the only way to build a
value; callers never observe which algorithm is in use.

### AD-3: Reference hash algorithm = SHA-256

| Option | Tradeoff | Decision |
|--------|----------|----------|
| Argon2/bcrypt/scrypt | KDF-resistant but heavy dep + tuning; overkill for high-entropy keys | rejected for reference |
| SHA-256 | Fast, `sha2` already in workspace, correct for high-entropy opaque keys | **chosen for reference** |

Rationale: API keys are machine-generated high-entropy secrets, not
user-chosen passwords — brute-force resistance comes from entropy, not a slow
KDF. SHA-256 is the **reference implementation's choice**; this is not a
normative SDK requirement. Resolver implementations MAY use stronger or
different algorithms while preserving the `ApiKeyHash` contract (opaque
construction + constant-time `verify`). Reuses the existing `sha2` dependency
— zero new crypto deps beyond `subtle`.

### AD-4: `Secret` zeroize = local newtype with `zeroize`

| Option | Tradeoff | Decision |
|--------|----------|----------|
| `secrecy` crate | Wraps + exposes; heavier API surface, transitive `zeroize` | rejected |
| Local newtype `Secret(Vec<u8>)` `#[derive(Zeroize, ZeroizeOnDrop)]` | One small dep, exact surface we need (`as_bytes`) | **chosen** |

`zeroize` is the minimal, audited primitive `secrecy` itself builds on. We
need only `as_bytes()` + drop-zeroing, so the newtype is a few lines. New dep:
`zeroize` (features = ["derive"]).

### AD-4b: Clock injection

`ApiKeyAuthenticationProvider` takes `Arc<dyn Clock>` for expiry evaluation.

| Tradeoff | Rationale |
|----------|-----------|
| `SystemTime::now()` inline | Untestable; non-deterministic in tests |
| `Arc<dyn Clock>` (mirrors JWT) | Deterministic tests with `FakeClock`; consistent with JWT authenticator constructor |

Constructor: `ApiKeyAuthenticationProvider::new(resolver, clock)`. Affects the
public constructor contract — callers must supply a clock. `service-sdk`'s
`RuntimeBuilder` already wires a system clock for JWT; same instance passes here.

### AD-5: `ApiKeyParser` default = split on first `.`

`DefaultApiKeyParser` splits `raw` on the **first** `.` occurrence via
`split_once('.')`. `{key_id}.{secret}` → the secret may itself contain dots;
only the first separator is significant. Empty key_id or empty secret →
`AuthenticationError::InvalidToken`. Malformed (no `.`) → `InvalidToken`.

### AD-6: `ApiKeyId` max length = 128 chars

Non-empty, `[a-zA-Z0-9_-]`, max **128**. Rationale: comfortably fits UUIDs,
ULIDs, prefixed ids (`ego_sk_<26-char-ulid>`), and short opaque handles, while
bounding the HashMap key size and rejecting pathological input before lookup.
Over-limit / bad-charset → `InvalidToken` (never leak which).

### AD-7: Scopes carrier = `Claims.custom["scopes"]`

`SecurityContext` has **no** `scopes` field (only `principal` + `claims`). The
proposal's "scopes on `SecurityContext`" maps to `Claims.custom["scopes"]` as
a JSON string array (`serde_json::Value::Array`). Rationale: scopes are
request-scoped auth assertions (Claims are exactly that, AD-002), not
principal-identity metadata (`Principal.attributes`). No SDK change needed.

## Data Flow

```
Credential::Bearer(raw)
   │  bearer_key()  ── non-Bearer / oversized → InvalidToken
   ▼
DefaultApiKeyParser::parse(raw) → (ApiKeyId, Secret)   ── malformed → InvalidToken
   ▼
resolver.lookup(&key_id) → Option<ApiKeyRecord>        ── None does NOT return early
   ▼
record.key_hash.verify(secret.as_bytes())  [subtle CT] ── always runs; a fixed dummy
   │                                                       hash stands in when the key
   │                                                       id is unknown, so "not found"
   │                                                       and "found, wrong secret" cost
   │                                                       the same time (no timing oracle)
   ▼
expires_at check (`now >= expires_at` ⇒ expired)       ── evaluated alongside the hash
   ▼
accept only if: found AND hash matched AND not expired ── any failure → InvalidToken
   ▼
SecurityContext { principal, claims{ custom["scopes"] } }
```

Every failure returns the SAME `AuthenticationError::InvalidToken` — no cause
differentiation (security invariant, MUST).

## File Changes

| File | Action | Description |
|------|--------|-------------|
| `crates/security-apikey/Cargo.toml` | Create | New crate manifest |
| `crates/security-apikey/src/lib.rs` | Create | `#![deny(missing_docs)]`, pub re-exports, layer note |
| `crates/security-apikey/src/key_id.rs` | Create | `ApiKeyId` validated value object (≤128, charset) |
| `crates/security-apikey/src/key_hash.rs` | Create | `ApiKeyHash` (opaque); `pub` SHA-256 constructor; `pub fn verify` (constant-time) |
| `crates/security-apikey/src/secret.rs` | Create | `Secret` newtype, zeroize-on-drop, `as_bytes` |
| `crates/security-apikey/src/parser.rs` | Create | `ApiKeyParser` trait + `DefaultApiKeyParser` |
| `crates/security-apikey/src/resolver.rs` | Create | `ApiKeyResolver`, `ApiKeyResolverError`, `ApiKeyRecord`, `InMemoryApiKeyResolver` |
| `crates/security-apikey/src/authenticator.rs` | Create | `ApiKeyAuthenticationProvider` impl `AuthenticationProvider` |
| `Cargo.toml` (workspace) | Modify | Add member `crates/security-apikey` |

## Interfaces / Contracts

```rust
pub struct ApiKeyId(String);              // ApiKeyId::new(s) -> Result<Self, AuthenticationError>
impl ApiKeyId { pub fn as_str(&self) -> &str; }

#[derive(Zeroize, ZeroizeOnDrop)]
pub struct Secret(Vec<u8>);
impl Secret { pub fn as_bytes(&self) -> &[u8]; }

// ApiKeyHash is fully opaque — no public fields, no algorithm enum.
pub struct ApiKeyHash(/* opaque */);
impl ApiKeyHash {
    pub fn sha256(digest: [u8; 32]) -> Self;       // public — lets external resolvers build ApiKeyRecord
    pub fn of(secret: &[u8]) -> Self;              // convenience: sha256(Sha256::digest(secret))
    pub fn verify(&self, secret: &[u8]) -> bool;   // constant-time (subtle); MUST contract
}

pub trait ApiKeyParser: Send + Sync {
    fn parse(&self, raw: &str) -> Result<(ApiKeyId, Secret), AuthenticationError>;
}
pub struct DefaultApiKeyParser;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ApiKeyResolverError {
    #[error("resolver backend unavailable: {0}")] Backend(String),
}

pub struct ApiKeyRecord {
    pub principal: Principal,
    pub scopes: Vec<String>,
    pub expires_at: Option<SystemTime>,
    pub metadata: HashMap<String, String>,
    pub key_hash: ApiKeyHash,
}

pub trait ApiKeyResolver: Send + Sync {   // object-safe, Arc<dyn>
    // Returns Arc<ApiKeyRecord> — cheap ref-count bump, no deep clone on the hot auth path.
    fn lookup(&self, key_id: &ApiKeyId) -> Result<Option<Arc<ApiKeyRecord>>, ApiKeyResolverError>;
}
pub struct InMemoryApiKeyResolver { /* HashMap<ApiKeyId, Arc<ApiKeyRecord>> */ }
pub type LocalApiKeyResolver = InMemoryApiKeyResolver;

pub struct ApiKeyAuthenticationProvider {          // impl AuthenticationProvider (sync)
    // resolver: Arc<dyn ApiKeyResolver>, parser: Arc<dyn ApiKeyParser>, clock: Arc<dyn Clock>
}
impl ApiKeyAuthenticationProvider {
    pub fn new(resolver: Arc<dyn ApiKeyResolver>, clock: Arc<dyn Clock>) -> Self; // DefaultApiKeyParser
    pub fn with_parser(self, parser: Arc<dyn ApiKeyParser>) -> Self;
}
```

`expires_at` compared against `Clock` (mirrors JWT's injected `Arc<dyn Clock>`
for deterministic tests). `MAX_KEY_BYTES` guard on the raw credential before
parsing, mirroring JWT's `MAX_TOKEN_BYTES`.

## Testing Strategy (strict TDD — `cargo test --workspace`)

| Layer | What to Test | Approach |
|-------|-------------|----------|
| Unit | `ApiKeyId` bounds: empty, 128, 129, bad charset | table assertions |
| Unit | `Secret` zeroized on drop | `Zeroize`/`ZeroizeOnDrop` derive + as_bytes roundtrip |
| Unit | `ApiKeyHash::verify` true/false; no early-exit | oracle-free stub, assert both branches run to completion |
| Unit | `DefaultApiKeyParser`: `a.b`, `a.b.c` (secret keeps dots), no dot, empty halves | table assertions |
| Unit | `InMemoryApiKeyResolver` lookup hit/miss; dual-key coexistence (2 slots, same principal) | HashMap fixtures |
| Unit | Provider flow: valid, unknown, expired, mismatch, malformed, non-Bearer → all `InvalidToken` (except valid) | fixed clock |
| Unit | Scopes appear in `SecurityContext.claims.custom["scopes"]` | assert JSON array |
| Contract | `Arc<dyn ApiKeyResolver>` object-safe; providers `Send + Sync` | compile assertions |
| Contract | `mockall` mock for `ApiKeyResolver` | `#[cfg_attr(test, mockall::automock)]` |

## Migration / Rollout

No migration required. Additive new crate, no existing callers.

## Integration Points

`service-sdk` runtime builder wires providers as `Arc<dyn AuthenticationProvider>`
selected by credential scheme via the extractor (same as JWT). Construction:
`Arc::new(ApiKeyAuthenticationProvider::new(resolver, clock))`. No builder API
change — it already accepts `Arc<dyn AuthenticationProvider>`.

## Open Questions

- [ ] AD-7 follow-up: if a first-class `SecurityContext::scopes` accessor is later desired, that is an additive SDK change, not part of CORE-021.
- [x] External resolver support: `ApiKeyHash::sha256`/`of` are `pub`, so external resolver crates (Postgres, Redis, Vault) can construct `ApiKeyHash` directly. No follow-up needed.
