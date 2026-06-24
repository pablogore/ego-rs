# Delta for security-sdk

## MODIFIED Requirements

### FR-001: Principal — gains tenant_id

`Principal` MUST carry: `kind: PrincipalKind`, `subject_id: SubjectId`, `tenant_id: Option<String>`, `roles: HashSet<Role>`, `attributes: HashMap<String, String>`.

(Previously: no `tenant_id`. AD-003 restricts Principal to identity-only. `claims: Vec<Claim>` removed — all claims live in `SecurityContext.claims`.)

- GIVEN `Principal::new(User, "user:1")`
- WHEN constructed
- THEN `tenant_id` is `None`, `roles` and `attributes` empty

- GIVEN tenant_id set to `"acme"`
- WHEN `principal.tenant_id()` is called
- THEN `Some("acme")` is returned

**Tests**: `constructs_with_required_fields`, `tenant_id_present`.

### FR-005: AuthenticationProvider — sync, returns SecurityContext

`AuthenticationProvider` MUST be object-safe, synchronous, and return `Result<SecurityContext, AuthenticationError>`:

```rust
fn authenticate(&self, credential: &Credential) -> Result<SecurityContext, AuthenticationError>;
```

(Previously: async trait returning `Result<Principal, SecurityError>`. AD-004: auth is CPU-bound, no I/O. Q7: AuthenticationProvider uses domain's `AuthenticationError` — `SecurityError` is reserved for authorization.)

- GIVEN a struct implementing `AuthenticationProvider`
- WHEN stored as `Arc<dyn AuthenticationProvider>`
- THEN compiles without trait-object safety errors

- GIVEN a provider that successfully authenticates
- WHEN `authenticate(credential)` returns `Ok(ctx)`
- THEN `ctx.principal()` and `ctx.claims()` are both populated

**Tests**: `provider_is_object_safe`, `returns_security_context`.

### FR-011: SecurityContext — requires Principal and Claims

`SecurityContext` MUST hold `principal: Principal` and `claims: Claims` (from `domain::auth`). Constructed as `SecurityContext::new(principal, claims)`. Exposes `principal()` and `claims()`. Clone + Send + Sync.

(Previously: no `claims` field. AD-002: claims are request-scoped, not persisted.)

- GIVEN a `Principal` and a `Claims`
- WHEN `SecurityContext::new(p, c)` is called
- THEN `ctx.principal()` returns the principal, `ctx.claims()` returns the claims

- GIVEN two tasks constructing contexts independently
- WHEN each accesses its own context
- THEN neither leaks state to the other

**Tests**: `constructs_from_principal_and_claims`, `no_ambient_state_leak`.

### NFR-005: No Ambient Security State

No code in `security-sdk` or `service-sdk` MUST store `SecurityContext` or `ServiceContext` in thread-local, task-local, or global storage (`static`, `OnceCell`, `LazyLock`, `once_cell`, `lazy_static`). Only explicit `ServiceContext` parameter passing is permitted.

(Previously: omitted `LazyLock`. AD-005 extends the prohibition.)

- GIVEN the workspace compiles
- WHEN `grep -rn "task_local.*ServiceContext\|CURRENT_CONTEXT" crates/`
- THEN zero matches

- GIVEN the workspace compiles
- WHEN `grep -rn "thread_local\|LazyLock\|once_cell" crates/security-sdk/src/ crates/service-sdk/src/context/`
- THEN zero matches for security/service context ambient storage

- GIVEN two independent tasks
- WHEN each constructs `SecurityContext::new(p, c)`
- THEN neither task sees the other's context, no global storage written

## ADDED Requirements

### FR-015: Claims integration — re-export from domain::auth

`SecurityContext.claims` MUST use `domain::auth::Claims` (`{ standard: StandardClaims, custom: BTreeMap<String, Value> }`). `security-sdk` MUST re-export `Claims` and `StandardClaims` so consumers avoid a direct `domain::auth` dependency.

- GIVEN ctx with `Claims { standard: _, custom: _ }`
- WHEN `ctx.claims().standard.iss` is accessed
- THEN it matches the constructed value

- GIVEN a crate depending only on `security-sdk`
- WHEN it writes `use ego_security_sdk::Claims`
- THEN it compiles

### FR-016: ServiceContext — security propagation field

`ServiceContext` MUST carry an `security: Option<SecurityContext>` field. The field is additive — all existing code compiles unchanged with the field defaulting to `None`. All access to authenticated identity and authorization flows exclusively through `ServiceContext`.

(Added by CORE-012. AD-007: explicit propagation via ServiceContext, no ambient state.)

- GIVEN a ServiceContext constructed without security providers
- WHEN `.security` is accessed
- THEN it returns `None`

- GIVEN a ServiceContext after successful authentication
- WHEN `.security` is accessed
- THEN it returns `Some(ctx)` where `ctx.principal()` and `ctx.claims()` are populated

**Tests**: `security_defaults_to_none`, `security_populated_after_auth`.
