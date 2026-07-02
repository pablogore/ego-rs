# Proposal: API Key Authentication (CORE-021)

## Intent

The ego-rs security stack has no first-class API key authentication provider.
Callers today must either reuse `BasicAuthenticationProvider` with synthetic
credentials or skip authentication entirely — both are incorrect for S2S,
B2B, and public-API consumers presenting opaque high-entropy keys.

This change adds `crates/security-apikey` — a self-contained crate that
implements `AuthenticationProvider` for opaque API keys, following the
established `security-jwt` pattern (resolver SPI + thin authenticator).

## Scope

### In Scope
- New crate `security-apikey` (`ego-security-apikey`) with:
  - `ApiKeyResolver` trait — SPI for key lookup, principal mapping, and scope
    attachment; zero storage knowledge in the crate
  - `ApiKeyResolverError` error type (mirrors `KeyResolverError`)
  - `ApiKeyAuthenticationProvider` struct implementing `AuthenticationProvider`
  - Constant-time comparison enforced inside the provider (not delegated to
    the resolver)
  - `InMemoryApiKeyResolver` reference implementation for tests and local dev
  - `LocalApiKeyResolver` alias or re-export of the in-memory impl
- Wire `Credential::Bearer` as the carrier for the raw key value (consistent
  with how `ApiKeyExtractor` today maps header values to `Credential::Bearer`)
- Propagate resolved scopes into `SecurityContext.claims.custom["scopes"]` as a
  JSON string array (opaque strings; enforcement is downstream). `SecurityContext`
  has no `scopes` field — `Claims.custom` is the correct carrier per AD-7.
- Unit tests covering happy path, unknown key, constant-time guarantee (timing
  oracle-free stub), `Send + Sync` bounds, and `mockall` mock for
  `ApiKeyResolver`

### Wire Format Contract

The SDK does not require a specific wire format, but the provider requires a
deterministic mapping from the presented credential to `(ApiKeyId, secret)`.
The default `ApiKeyAuthenticationProvider` documents a recommended split
strategy (e.g. `{key_id}.{secret}`); callers may supply a custom parser.
Key generation, prefixes (`ego_sk_...`), and entropy management remain the
caller's concern.

### Out of Scope
- Key generation utilities or prefix enforcement (`ego_sk_...`) — caller concern
- Grace-period / dual-key rotation logic — owned by `ApiKeyResolver` implementor
- Storage adapters (Postgres, Redis, Vault) — downstream crates
- Authorization / scope enforcement — `AuthorizationProvider`, not this crate
- Changes to `ego-domain` `Credential` enum — `Credential::Bearer` covers the
  wire format without a new variant
- Changes to `security-sdk` public API — new crate is additive

## Capabilities

### New Capabilities
- `api-key-authentication`: `ApiKeyId` value object, `ApiKeyResolver` SPI (`lookup(&ApiKeyId)`),
  `ApiKeyRecord` (principal, scopes, expires_at, metadata, key_hash),
  `ApiKeyAuthenticationProvider` with deterministic 4-step validation flow,
  `InMemoryApiKeyResolver` (tests/local dev only — no persistence, no distributed sync),
  constant-time comparison, scope propagation

### Modified Capabilities
None

## Approach

Mirror `security-jwt` structure exactly:

```
crates/security-apikey/
  src/
    lib.rs            — pub re-exports
    key_id.rs         — ApiKeyId value object (validated, constrained)
    key_hash.rs       — ApiKeyHash value object (opaque, constant-time verify)
    secret.rs         — Secret value object (zeroize on drop)
    parser.rs         — ApiKeyParser trait + DefaultApiKeyParser (dot-separator)
    resolver.rs       — ApiKeyResolver trait + ApiKeyResolverError + ApiKeyRecord + InMemoryApiKeyResolver
    authenticator.rs  — ApiKeyAuthenticationProvider (implements AuthenticationProvider)
  Cargo.toml
```

### Key types

```rust
/// Validated lookup token. Invariants: non-empty, bounded length (exact limit is
/// a design-phase decision), restricted character set [a-zA-Z0-9_-].
pub struct ApiKeyId(String);

/// Opaque hash value. Internal representation (PHC string, raw bytes + algorithm
/// tag, etc.) is a design-phase decision. Exposes a single constant-time
/// verification method so the provider never touches the algorithm directly.
pub struct ApiKeyHash(/* opaque */);

impl ApiKeyHash {
    /// Constant-time verification. Returns true iff `secret` matches the stored hash.
    /// MUST perform the comparison in constant time — this is a contract obligation,
    /// not an implementation detail.
    pub fn verify(&self, secret: &[u8]) -> bool;
}

/// Secret bytes extracted from the raw credential. `as_bytes()` returns `&[u8]`
/// without copying or exposing a `String`. Zeroized on drop (design phase will
/// decide whether to use `secrecy` or a local newtype).
pub struct Secret(/* opaque */);

impl Secret {
    pub fn as_bytes(&self) -> &[u8];
}

pub struct ApiKeyRecord {
    pub principal: Principal,              // carries tenant_id per CORE-012 design
    pub scopes: Vec<String>,               // opaque strings; enforcement is downstream
    pub expires_at: Option<SystemTime>,    // None = no expiry
    pub metadata: HashMap<String, String>,
    // Providers MUST ignore unknown metadata entries.
    // Consumers MUST NOT depend on provider-specific metadata keys unless
    // explicitly documented by the resolver implementation.
    pub key_hash: ApiKeyHash,              // constant-time verification via ApiKeyHash::verify
}

/// SPI for key-format parsing. The default impl splits on the first `.`
/// (e.g. `{key_id}.{secret}`). Callers supply a custom impl for other formats.
pub trait ApiKeyParser: Send + Sync {
    fn parse(&self, raw: &str) -> Result<(ApiKeyId, Secret), AuthenticationError>;
}

pub trait ApiKeyResolver: Send + Sync {
    fn lookup(&self, key_id: &ApiKeyId) -> Result<Option<ApiKeyRecord>, ApiKeyResolverError>;
}
```

### Provider flow (strict order)

```
raw_key (from Credential::Bearer)
    │
    ▼
Provider: parse(raw_key) → (ApiKeyId, secret)   // rejects malformed → InvalidToken
    │
    ▼
resolver.lookup(&key_id) → Option<ApiKeyRecord>  // None does NOT return early
    │
    ▼
record.key_hash.verify(secret.as_bytes())
    // Runs unconditionally — a fixed dummy hash stands in when lookup returned
    // None, so "key not found" and "key found, wrong secret" cost the same
    // time. Returning early on None here would be a timing oracle.
    // ApiKeyHash::verify owns the algorithm — constant-time guaranteed by the
    // value object, not the caller. The resolver is responsible for hash
    // algorithm selection and consistency when constructing ApiKeyHash.
    // The SDK does not prescribe Argon2, bcrypt, or any other algorithm.
    │
    ▼
expires_at check: expired → InvalidToken
    │
    ▼
Accept only if: found AND hash matched AND not expired — otherwise InvalidToken
    │
    ▼
SecurityContext { principal, claims.custom["scopes"] }
```

### Security invariant (MUST)

> The provider MUST return `AuthenticationError::InvalidToken` for unknown,
> expired, revoked, malformed, or hash-mismatched API keys — without
> distinguishing the failure cause.

This is a security property. No future change may introduce differentiated
errors at this layer.

### AD-004 contract

Same as `KeyResolver`: `ApiKeyResolver::lookup` MUST be cache-first; no I/O
inside `authenticate`. `InMemoryApiKeyResolver` is the canonical reference
implementation. A resolver that does I/O inside `lookup` will compile — the
contract is enforced by documentation and the reference impl, not the type
system (same trade-off accepted for `KeyResolver`).

## Affected Areas

| Area | Impact | Description |
|------|--------|-------------|
| `crates/security-apikey/` | New | Entire new crate |
| `Cargo.toml` (workspace) | Modified | Add member `crates/security-apikey` |
| `crates/security-sdk/src/lib.rs` | None | No changes required; crate is additive |

## Risks

| Risk | Likelihood | Mitigation |
|------|------------|------------|
| Timing side-channel via early-exit comparison | Med | Provider enforces `subtle::ConstantTimeEq` after expiry check; unit test with oracle-free stub |
| Resolver without an indexed ApiKeyId lookup | Low | Resolver implementations SHOULD maintain an indexed lookup by `ApiKeyId`; `InMemoryApiKeyResolver` uses a `HashMap<ApiKeyId, ApiKeyRecord>` as the reference |
| Custom `ApiKeyParser` producing ambiguous mappings | Low | Default parser is canonical; custom parsers MUST produce a deterministic `(ApiKeyId, Secret)` mapping — documented as a contract obligation of the SPI |
| `Credential::Bearer` ambiguity (JWT vs API key) | Low | Consumers choose the provider; pipeline routes by credential scheme via extractor |
| Scope format diverges between teams | Low | Scopes are opaque strings; format convention is a caller contract, not SDK contract |
| AD-004 violated by resolver doing I/O | Med | Doc contract mirrors `KeyResolver`; `InMemoryApiKeyResolver` is the canonical cache-first reference |

## Rollback Plan

`security-apikey` is a new crate with no existing callers. Rollback = remove
the crate from the workspace `members` list and delete `crates/security-apikey/`.
No existing crates are modified.

## Dependencies

- `subtle` crate for constant-time comparison (no existing dep; lightweight, no unsafe surface of its own)
- `ego-security-sdk` (workspace path dep) for `AuthenticationProvider`, `SecurityContext`, `Credential`
- `ego-domain` (workspace path dep) for `AuthenticationError`, `Principal`
- `mockall` (dev-dependency, already used across the workspace)

## Success Criteria

- [ ] `cargo test --workspace` passes with all new tests green
- [ ] `ApiKeyResolver` is object-safe (`Arc<dyn ApiKeyResolver>` compiles)
- [ ] `ApiKeyAuthenticationProvider` implements `AuthenticationProvider` and passes
      `AuthenticationProvider` object-safety and `Send + Sync` tests
- [ ] Constant-time comparison path is exercised (no short-circuit on mismatch)
- [ ] Reference resolver demonstrates dual-key coexistence: same principal,
      two simultaneous valid key slots, both accepted (caller-owned rotation pattern)
- [ ] Unknown key returns `AuthenticationError::InvalidToken` (same error as mismatch)
- [ ] Scopes from the resolver appear in `SecurityContext.claims.custom["scopes"]` as a JSON string array
