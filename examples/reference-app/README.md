# reference-app

The **Production Reference Service** (CORE-018) — a dogfooding milestone proving ego-rs's public APIs compose into a real, if minimal, business capability: registering a user into a tenant organization, end to end, over a real HTTP server.

Built using *only* ego-rs's public surface:

- **Application composition** — `App::builder()` (CORE-028's canonical composition path; `AppBuilder` delegates to the lower-level `RuntimeBuilder` internally)
- **Config + Logging** — `kit-config`-materialized `LoggingSettings` → `build_logger` → `App::builder().logger(...)`
- **Security / JWT** — `Hs256AuthenticationProvider`, `#[authorize]`, `#[tenant_scoped]`
- **Tenant enforcement** — fail-closed by default; the query route checks the authenticated principal's tenant against the requested one
- **CQRS read-side engine** — a real `UsersByTenant` projection (not the unrelated DI `resolve_projection`), with per-tenant tag isolation
- **TestKit** — guard-chain and fixture-based tests throughout

## Layout (hexagonal)

```
src/
├── domain/          User, TenantOrganization — PersistentEntity aggregates
├── application.rs   RegisterUser trait + impl — the use-case core, framework-agnostic
├── ports/http/      Concrete routes, handlers, Swagger/OpenAPI (the HTTP adapter)
├── read_side/       UsersByTenant projection wiring on the real CQRS engine
└── main.rs          Entry point: builds the runtime, serves HTTP, graceful shutdown
```

## Run it

```bash
cargo run -p reference-app
# reference-app: listening on 127.0.0.1:3000
```

### Register a user

Requests must carry a bearer JWT (HS256, signed with the dev-only key in `src/lib.rs::DEV_SIGNING_KEY`) whose `tenant_id` claim matches the request body's `tenant_id`:

```bash
python3 - <<'PY'
import base64, hmac, hashlib, json, time

def b64url(data: bytes) -> str:
    return base64.urlsafe_b64encode(data).rstrip(b'=').decode()

header = {"alg": "HS256", "typ": "JWT"}
payload = {"sub": "user-1", "exp": int(time.time()) + 3600, "tenant_id": "tenant-a"}
key = b"reference-app-development-signing-key-not-for-prod"
signing_input = b64url(json.dumps(header, separators=(',', ':')).encode()) + "." + b64url(json.dumps(payload, separators=(',', ':')).encode())
sig = hmac.new(key, signing_input.encode(), hashlib.sha256).digest()
print(signing_input + "." + b64url(sig))
PY
```

```bash
TOKEN="<paste the token printed above>"

curl -i -X POST http://127.0.0.1:3000/register \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer $TOKEN" \
  -d '{"user_id":"user-1","email":"user@example.com","tenant_id":"tenant-a","org_name":"Acme"}'
# HTTP/1.1 201 Created
# {"user_id":"user-1","tenant_id":"tenant-a"}
```

### Query the read-side projection

```bash
curl -i http://127.0.0.1:3000/tenants/tenant-a/users \
  -H "Authorization: Bearer $TOKEN"
```

### Swagger / OpenAPI

- Interactive docs: http://127.0.0.1:3000/swagger-ui
- Raw spec: http://127.0.0.1:3000/api-docs/openapi.json

### Local state

External effects (currently: the post-registration "welcome email") are persisted durably in an embedded [Stoolap](https://github.com/stoolap/stoolap) store on disk — no separate server to run, unlike the Postgres-backed event stores above. Defaults to `data/effects` inside this crate; override with `EGO_REFERENCE_APP_EFFECT_STORE_PATH`. Delete the directory to reset all accepted-but-undelivered effect state (never committed — see `.gitignore`).

### Shutting down

`Ctrl-C` triggers the full graceful sequence: drain in-flight HTTP requests → stop and drain the read-side scheduler → drain the runtime's sync teardown stack (logger/security). Each step is logged.

## Testing it

```bash
cargo test -p reference-app
```

Covers the guard chain (authz/tenant-scoping denial, happy path), the documented non-atomic dual-write (org-first, no saga — a `User`-write failure after the org write succeeds leaves a benign, reusable orphan org, not a rollback), observability event recording, the read-side projection (including tenant isolation), and a real end-to-end HTTP round trip against a live `axum::serve()` socket.

## Non-goals

No saga/compensation for the dual write, no production (non-`Noop`) observability adapter, no gRPC, no admin UI, no multi-region/clustering. See `openspec/changes/archive/` (once archived) for the full spec and design rationale.
