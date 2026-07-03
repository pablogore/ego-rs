# Delta for Security API Key

## ADDED Requirements

### Requirement: ApiKeyId Value Object

`ApiKeyId` MUST accept only non-empty strings composed exclusively of
`[a-zA-Z0-9_-]` characters. `ApiKeyId` MUST reject any other character and
MUST reject the empty string. The maximum allowed length is a design-phase
decision; the spec requires only that a hard upper bound exists and is enforced
at construction time.

#### Scenario: Valid identifier accepted

- GIVEN a string composed of `[a-zA-Z0-9_-]`, length within the allowed bound
- WHEN `ApiKeyId::new` (or equivalent constructor) is called
- THEN the call succeeds and the stored value equals the input

#### Scenario: Empty string rejected

- GIVEN an empty string
- WHEN `ApiKeyId::new` is called
- THEN the call returns an error; no `ApiKeyId` is produced

#### Scenario: Forbidden character rejected

- GIVEN a string containing a character outside `[a-zA-Z0-9_-]` (e.g. `@`, `.`, space)
- WHEN `ApiKeyId::new` is called
- THEN the call returns an error; no `ApiKeyId` is produced

---

### Requirement: Secret Value Object

`Secret` MUST expose the secret bytes only via `as_bytes() -> &[u8]`. `Secret`
MUST NOT expose a `String` representation. `Secret` MUST zero its memory on
drop (zeroized-on-drop guarantee).

#### Scenario: Bytes accessible

- GIVEN a `Secret` constructed from raw bytes `b`
- WHEN `secret.as_bytes()` is called
- THEN the returned slice equals `b`

#### Scenario: Zeroized on drop

- GIVEN a `Secret` holding non-zero bytes
- WHEN the `Secret` is dropped
- THEN the underlying memory is overwritten with zeros before deallocation

---

### Requirement: ApiKeyHash Constant-Time Verification

`ApiKeyHash::verify(&self, secret: &[u8]) -> bool` MUST perform the comparison
in constant time with respect to the secret bytes — no early exit on the first
differing byte. The comparison algorithm is a design-phase decision; the spec
requires only that constant time is guaranteed and that the algorithm is
encapsulated inside `ApiKeyHash` (the provider MUST NOT access it directly).

#### Scenario: Matching secret returns true

- GIVEN an `ApiKeyHash` constructed from secret bytes `s`
- WHEN `verify(s)` is called
- THEN the method returns `true`

#### Scenario: Non-matching secret returns false

- GIVEN an `ApiKeyHash` constructed from secret bytes `s`
- WHEN `verify(t)` is called where `t != s`
- THEN the method returns `false` and does NOT exit early (constant-time path)

---

### Requirement: ApiKeyRecord Structure

`ApiKeyRecord` MUST contain: `principal: Principal`, `scopes: Vec<String>`,
`expires_at: Option<SystemTime>`, `metadata: Arc<HashMap<String, String>>`,
`key_hash: ApiKeyHash`. Resolver implementations MUST ignore unknown `metadata`
entries. Consumers MUST NOT depend on provider-specific metadata keys unless
explicitly documented by the resolver.

#### Scenario: Record with no expiry

- GIVEN an `ApiKeyRecord` where `expires_at` is `None`
- WHEN the provider processes it
- THEN the expiry check is skipped (no expiry enforced)

#### Scenario: Record metadata ignored when unknown

- GIVEN an `ApiKeyRecord` with a `metadata` key not documented by the resolver
- WHEN the resolver returns that record and the provider uses it
- THEN the provider does not fail or expose that key

---

### Requirement: MAX_KEY_BYTES Guard

The provider MUST reject raw credentials larger than `MAX_KEY_BYTES` with
`AuthenticationError::InvalidToken` before any parsing occurs. The exact
constant is a design-phase decision; the spec requires only that a hard upper
bound is enforced before the parser is invoked.

#### Scenario: Oversized credential rejected

- GIVEN a `Credential::Bearer` whose raw string length exceeds `MAX_KEY_BYTES`
- WHEN `authenticate` is called
- THEN `Err(AuthenticationError::InvalidToken)` is returned without calling the parser

#### Scenario: Credential at the limit accepted (proceeds to parsing)

- GIVEN a `Credential::Bearer` whose raw string length equals `MAX_KEY_BYTES`
- WHEN `authenticate` is called
- THEN the guard passes and parsing is attempted (outcome depends on format)

---

### Requirement: ApiKeyParser Deterministic Parsing

`ApiKeyParser: Send + Sync` MUST be deterministic: the same `raw` string MUST
always produce the same `(ApiKeyId, Secret)` result. Custom parsers MUST satisfy
this contract. A malformed raw string MUST produce
`Err(AuthenticationError::InvalidToken)`, not panic.

#### Scenario: Valid raw key parsed

- GIVEN a raw key string in the format understood by the parser (e.g. `{id}.{secret}`)
- WHEN `parser.parse(raw)` is called
- THEN `Ok((ApiKeyId, Secret))` is returned with the correct split

#### Scenario: Malformed raw key rejected

- GIVEN a raw key string that does not match the expected format
- WHEN `parser.parse(raw)` is called
- THEN `Err(AuthenticationError::InvalidToken)` is returned (no panic)

#### Scenario: Determinism

- GIVEN the same raw string `r` and the same parser instance
- WHEN `parse(r)` is called twice
- THEN both calls return identical `(ApiKeyId, Secret)` values

---

### Requirement: ApiKeyResolver Cache-First Contract (AD-004)

`ApiKeyResolver: Send + Sync`. `lookup(&self, key_id: &ApiKeyId) ->
Result<Option<ApiKeyRecord>, ApiKeyResolverError>` MUST return from locally
available state without performing I/O on the calling thread inside
`authenticate`, on any path, including the not-found path.
`InMemoryApiKeyResolver` is the canonical reference implementation
(HashMap-backed, no I/O, no persistence, no distributed sync).

This is a hard requirement, not a performance preference: the Provider
Validation Flow requirement below deliberately performs equal work (a
hash-verify with a dummy digest) whether `lookup` returns `Some` or `None`,
specifically so an unknown key id and a known key id with the wrong secret
are indistinguishable by response time. If `lookup` itself has different
latency for a hit vs. a miss (a database round-trip, an HTTP call, lock
contention), that timing difference reopens the same side-channel the
dummy-hash step exists to close, regardless of how `key_hash.verify`
behaves. Rust's type system cannot enforce this at compile time;
implementors MUST satisfy it by construction (in-memory maps or warmed
local caches only, never a pass-through to a remote store).

#### Scenario: Known key returned

- GIVEN an `ApiKeyResolver` holding key id `K`
- WHEN `lookup(&K)` is called
- THEN `Ok(Some(record))` is returned synchronously from in-memory state

#### Scenario: Unknown key returns None

- GIVEN an `ApiKeyResolver` that does not hold key id `K`
- WHEN `lookup(&K)` is called
- THEN `Ok(None)` is returned

#### Scenario: Object-safety

- GIVEN a resolver implementing `ApiKeyResolver`
- WHEN it is stored as `Arc<dyn ApiKeyResolver>`
- THEN it compiles without error

---

### Requirement: Provider Validation Flow (Strict Order)

`ApiKeyAuthenticationProvider` MUST execute validation in this exact sequence:

1. Extract raw string from `Credential::Bearer`; any other credential variant MUST return `Err(AuthenticationError::InvalidToken)`.
2. Call `parser.parse(raw)` → `(ApiKeyId, Secret)`; failure MUST return `Err(AuthenticationError::InvalidToken)`.
3. Call `resolver.lookup(&key_id)` → `Option<ApiKeyRecord>`. `None` MUST NOT return early.
4. Call `key_hash.verify(secret.as_bytes())` unconditionally — using the found record's hash, or a fixed placeholder hash when `lookup` returned `None` — so an unknown key id and a known key id with the wrong secret perform identical work (no timing distinction between "key not found" and "key found, wrong secret").
5. If a record was found, evaluate `record.expires_at`: `None` never expires; `Some(t)` MUST be treated as expired when `now >= t` (the boundary instant `now == t` is expired, not valid).
6. Return `Ok(SecurityContext { principal, claims.custom["scopes"] })` only if a record was found AND the hash matched AND the record is not expired. Otherwise return `Err(AuthenticationError::InvalidToken)` uniformly — which condition failed MUST NOT be distinguishable from the returned error or from response timing.

#### Scenario: Happy path

- GIVEN a valid raw key, resolver returns a matching non-expired record, hash matches
- WHEN `authenticate(Credential::Bearer(raw_key))` is called
- THEN `Ok(SecurityContext)` is returned with the record's principal and scopes

#### Scenario: Non-Bearer credential

- GIVEN `Credential::Basic { username, secret }`
- WHEN `authenticate` is called
- THEN `Err(AuthenticationError::InvalidToken)` is returned

#### Scenario: Malformed raw key (parse failure)

- GIVEN a raw key string that the parser cannot parse
- WHEN `authenticate` is called
- THEN `Err(AuthenticationError::InvalidToken)` is returned

#### Scenario: Unknown key id (resolver returns None)

- GIVEN a correctly formatted raw key whose id is not in the resolver
- WHEN `authenticate` is called
- THEN `Err(AuthenticationError::InvalidToken)` is returned

#### Scenario: Expired record

- GIVEN a correctly formatted raw key, resolver returns a record with `expires_at < now`
- WHEN `authenticate` is called
- THEN `Err(AuthenticationError::InvalidToken)` is returned

#### Scenario: Hash mismatch

- GIVEN a correctly formatted raw key, resolver returns a non-expired record, but `key_hash.verify` returns `false`
- WHEN `authenticate` is called
- THEN `Err(AuthenticationError::InvalidToken)` is returned

---

### Requirement: Uniform Error — No Failure-Cause Disclosure

The provider MUST return `AuthenticationError::InvalidToken` for every failure
case (unknown, expired, revoked, malformed, hash-mismatched). The provider
MUST NOT return different error variants to distinguish failure causes. This
requirement is permanent; no future change may weaken it.

#### Scenario: All failure paths collapse to InvalidToken

- GIVEN any of: unknown key, expired key, hash mismatch, malformed raw, wrong credential type
- WHEN `authenticate` is called
- THEN the returned error is `AuthenticationError::InvalidToken` in every case

---

### Requirement: Scope Propagation

The provider MUST propagate the `scopes` from the resolved `ApiKeyRecord` into
`SecurityContext.claims.custom["scopes"]` as a JSON string array
(`serde_json::Value::Array`). `SecurityContext` has no `scopes` field; `Claims.custom`
is the carrier (AD-7). Scopes are opaque strings; the provider MUST NOT validate,
transform, or filter them.

#### Scenario: Scopes propagated

- GIVEN a record with `scopes = ["read:orders", "write:invoices"]`
- WHEN authentication succeeds
- THEN `SecurityContext.claims.custom["scopes"]` equals `["read:orders", "write:invoices"]` as a JSON array

#### Scenario: Empty scopes propagated as empty

- GIVEN a record with `scopes = []`
- WHEN authentication succeeds
- THEN `SecurityContext.claims.custom["scopes"]` is an empty JSON array (not an error)

---

### Requirement: Send + Sync and Object Safety

`ApiKeyAuthenticationProvider` MUST implement `Send + Sync`. `ApiKeyResolver`
MUST be object-safe and storable as `Arc<dyn ApiKeyResolver>`.
`ApiKeyAuthenticationProvider` MUST implement `AuthenticationProvider` and MUST
be storable as `Arc<dyn AuthenticationProvider>`.

#### Scenario: Provider Send + Sync

- GIVEN `ApiKeyAuthenticationProvider`
- WHEN tested with a compile-time `fn assert_send_sync<T: Send + Sync>()` assertion
- THEN it compiles

#### Scenario: Provider object-safety

- GIVEN an `ApiKeyAuthenticationProvider` instance
- WHEN stored as `Arc<dyn AuthenticationProvider>`
- THEN it compiles without error

---

### Requirement: InMemoryApiKeyResolver — Dual-Key Coexistence

`InMemoryApiKeyResolver` MUST support registering two simultaneous valid key
slots for the same principal. Both slots MUST be independently resolvable by
their respective `ApiKeyId`. This enables the caller-owned rotation pattern
(old key valid until caller rotates).

#### Scenario: Two keys for same principal both accepted

- GIVEN two `ApiKeyRecord`s with the same principal but different `ApiKeyId`s, both registered
- WHEN each raw key is presented to the provider in sequence
- THEN both succeed and return a `SecurityContext` for the same principal
