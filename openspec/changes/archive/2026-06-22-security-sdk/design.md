# Design: Security SDK

## Architecture Overview

`security-sdk` is a **cross-cutting SDK crate** (`crates/security-sdk`, package name `ego-security-sdk`) that holds canonical security primitives. It is a sibling of `service-sdk` and is NOT a member of any layer (domain/application/infrastructure/transport). It depends on **no ego crate** — only on third-party libraries. This keeps the strict layer rules intact: any layer may import `security-sdk` without risking a cycle, because `security-sdk` imports nobody.

`service-sdk` becomes the **first consumer**: it gains a dependency on `security-sdk` and wires `SecurityContext` into `ServiceContext` as an additive optional field.

```
            ┌─────────────────────────────────────────────┐
            │              security-sdk                     │
            │  (no ego deps — only async-trait, thiserror,  │
            │   serde)                                      │
            │                                               │
            │  contracts:  AuthenticationProvider           │
            │              AuthorizationProvider            │
            │              RoleStore                         │
            │  models:     Principal / Credential /          │
            │              SecurityContext / AccessRequest  │
            │  providers:  Basic /                          │
            │              Rbac + InMemoryRoleStore         │
            └───────────────────────▲───────────────────────┘
                                     │ depends on (one-way)
            ┌───────────────────────┴───────────────────────┐
            │                service-sdk                     │
            │  ServiceContext { …, security:                 │
            │                    Option<Arc<SecurityContext>>}│
            │  RuntimeBuilder propagates `security` unchanged │
            └────────────────────────────────────────────────┘
```

**Dependency direction is one-way and strict**: `service-sdk → security-sdk`. Never the reverse. Transports (HTTP/gRPC, future) translate their wire auth into `Credential` and call providers; the Security SDK ships none of that.

### Call shape (what the future macro targets)

```
transport edge ──translate──▶ Credential
        │
        ▼
AuthenticationProvider::authenticate(&Credential) ──▶ Principal
        │  (Principal placed into SecurityContext, attached to ServiceContext)
        ▼
ServiceContext { security: Some(Arc<SecurityContext>) } ──flows through graph──▶
        │
        ▼
authorize_in_context(&ServiceContext, Resource, Action, &dyn AuthorizationProvider)
        │  resolve ctx.security → build AccessRequest → provider.authorize → map Deny
        ▼
Result<(), SecurityError>
```

[Full design document contents preserved from openspec/changes/security-sdk/design.md — see original spec.md for complete technical details including module structure, trait signatures, core types, provider designs, ServiceContext integration, testing strategy, and Cargo.toml changes.]
