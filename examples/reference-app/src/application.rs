//! `RegisterUser` — the guarded, dual-write service operation (design.md
//! AD-4, AD-5, AD-7).
//!
//! Registered/resolved via the CORE-025 canonical trait-proxy path
//! (`RuntimeBuilder::with_service` / `Runtime::resolve`), not `Injectable`
//! (AD-7: the Injectable-XOR-resolvable-proxy constraint).
//!
//! Orchestrates two independent `EntityRuntime`s (AD-4 — one per aggregate,
//! no shared event enum): ensures the `TenantOrganization` first (idempotent),
//! then registers the `User` (AD-5's org-first non-atomic dual write). On a
//! `User`-write failure after the `TenantOrganization` write already
//! succeeded, this returns `Err` and leaves the org persisted — no
//! compensating delete, no saga (documented, not hidden).

use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
use ego_domain::{DomainEvent, Observability, SemanticEvent};
use ego_security_sdk::SecurityError;
use ego_service_sdk::context::ServiceContext;
use ego_service_sdk::error::category::ErrorCategory;
use ego_service_sdk::error::ServiceErrorTrait;
#[allow(unused_imports)]
use ego_service_sdk_macros::{authorize, operation, service, tenant_scoped};
use persistent_entity::command_context::CommandContext;
use persistent_entity::entity_ref::EntityRef;
use persistent_entity::error::EntityError;
use persistent_entity::persistent_entity::CommandResult;
use persistent_entity::runtime::EntityRuntime;
use serde::{Deserialize, Serialize};

use crate::domain::tenant_org::{
    OrganizationEnsured, TenantOrgCommand, TenantOrgState, TenantOrganizationEntity,
};
use crate::domain::user::{UserCommand, UserEntity, UserRegistered, UserState};
use crate::read_side::ReadSideSink;

/// Input to `RegisterUser::register`. Also the OpenAPI request body schema
/// for `POST /register` (`ports::http`) — one struct serving both roles;
/// see `ports::http`'s module doc for why this isn't split into a separate
/// DTO.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, utoipa::ToSchema)]
pub struct RegisterInput {
    pub user_id: String,
    pub email: String,
    pub tenant_id: String,
    pub org_name: String,
}

/// Output of a successful registration. Also the OpenAPI response body
/// schema for `POST /register`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, utoipa::ToSchema)]
pub struct RegisterOutput {
    pub user_id: String,
    pub tenant_id: String,
}

/// `RegisterUser`'s error type — required by `#[authorize]`/`#[tenant_scoped]`
/// to carry `From<SecurityError>` for guard denials, plus the dual write's
/// own `EntityError` outcomes (AD-5's non-atomic partial failure).
#[derive(Debug)]
pub enum RegisterUserError {
    /// A guard (`#[authorize]`/`#[tenant_scoped]`) denied the call. Carries
    /// the original `SecurityError` (not a stringified copy) so transport
    /// can map it to the correct status code (401/403/500) via
    /// `ego_transport`'s existing granular `From<SecurityError> for
    /// TransportError`, instead of collapsing every denial into 403.
    Security(SecurityError),
    /// A `TenantOrganization` or `User` entity write failed.
    EntityWrite(String),
}

impl std::fmt::Display for RegisterUserError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RegisterUserError::Security(e) => write!(f, "security error: {e}"),
            RegisterUserError::EntityWrite(m) => write!(f, "entity write error: {m}"),
        }
    }
}

impl From<SecurityError> for RegisterUserError {
    fn from(e: SecurityError) -> Self {
        RegisterUserError::Security(e)
    }
}

impl From<EntityError> for RegisterUserError {
    fn from(e: EntityError) -> Self {
        RegisterUserError::EntityWrite(e.to_string())
    }
}

impl ServiceErrorTrait for RegisterUserError {
    fn code(&self) -> &str {
        match self {
            RegisterUserError::Security(_) => "REGISTER_USER_SECURITY_ERROR",
            RegisterUserError::EntityWrite(_) => "REGISTER_USER_ENTITY_WRITE_ERROR",
        }
    }

    fn category(&self) -> ErrorCategory {
        match self {
            RegisterUserError::Security(_) => ErrorCategory::Authorization,
            RegisterUserError::EntityWrite(_) => ErrorCategory::System,
        }
    }

    fn message(&self) -> String {
        self.to_string()
    }
}

/// Registers a `User` within a `TenantOrganization` (design.md's reference
/// journey). Guarded by both `#[authorize]` and `#[tenant_scoped]` — either
/// denial prevents any entity write (reference-service spec: "Unauthorized
/// principal denied", "Cross-tenant request denied").
#[service(version = "1.0.0")]
pub trait RegisterUser {
    #[operation]
    #[authorize(context = ctx, permission = "user:register")]
    #[tenant_scoped]
    async fn register(
        &self,
        ctx: ServiceContext,
        input: RegisterInput,
    ) -> Result<RegisterOutput, RegisterUserError>;
}

/// The concrete implementation (AD-4: two independent `EntityRuntime`s).
pub struct RegisterUserImpl {
    org_runtime: Arc<EntityRuntime<OrganizationEnsured>>,
    user_runtime: Arc<EntityRuntime<UserRegistered>>,
    observability: Option<Arc<dyn Observability>>,
    /// Bridges real emitted domain events into the `UsersByTenant` read-side
    /// projection (new capability, layered on CORE-005's engine — see
    /// `crate::read_side`). `None` by default so every existing
    /// `RegisterUserImpl::new(...)` call site keeps compiling unchanged;
    /// `build_runtime` wires a real sink via `with_read_side_sink`.
    read_side_sink: Option<ReadSideSink>,
}

impl RegisterUserImpl {
    pub fn new(
        org_runtime: Arc<EntityRuntime<OrganizationEnsured>>,
        user_runtime: Arc<EntityRuntime<UserRegistered>>,
        observability: Option<Arc<dyn Observability>>,
    ) -> Self {
        Self {
            org_runtime,
            user_runtime,
            observability,
            read_side_sink: None,
        }
    }

    /// Wires a `ReadSideSink` so successful writes also feed the
    /// `UsersByTenant` read-side projection.
    pub fn with_read_side_sink(mut self, sink: ReadSideSink) -> Self {
        self.read_side_sink = Some(sink);
        self
    }

    /// Records a business-outcome event (CORE-012A). Guard denials are
    /// already recorded for free by `RuntimeBuilder::with_observability`'s
    /// macro-guard wiring — only the two outcomes reached from inside this
    /// method's own body (success, partial-failure) need an explicit call.
    fn record(&self, event_name: &str, actor_id: &str, lifecycle_state: &str) {
        if let Some(obs) = &self.observability {
            if let Ok(event) = SemanticEvent::without_metadata(
                event_name,
                actor_id,
                actor_id,
                lifecycle_state,
                Utc::now().to_rfc3339(),
            ) {
                obs.trace(event);
            }
        }
    }

    /// Feeds real emitted domain events into the read-side sink (a no-op
    /// when none is wired). Only called for `CommandResult::Events` — the
    /// `NoEvents` (idempotent no-op) case has nothing new to project.
    fn publish_read_side<E: DomainEvent>(&self, tenant_id: &str, events: &[E]) {
        if let Some(sink) = &self.read_side_sink {
            for event in events {
                sink.record(
                    tenant_id,
                    event.aggregate_id(),
                    event.event_type(),
                    event.payload().clone(),
                    *event.occurred_at(),
                );
            }
        }
    }
}

#[async_trait]
impl RegisterUser for RegisterUserImpl {
    async fn register(
        &self,
        ctx: ServiceContext,
        input: RegisterInput,
    ) -> Result<RegisterOutput, RegisterUserError> {
        // Security: `#[tenant_scoped]` only proves `ctx` resolves to SOME
        // tenant — it never compares that against `input.tenant_id` (a
        // client-controlled request-body field). Without this check, an
        // authenticated caller from tenant A could submit `tenant_id:
        // "tenant-B"` and write into tenant B. Reject the request outright
        // rather than trusting `input.tenant_id` as the write-tenant.
        //
        // Fail closed: a resolved canonical tenant that equals `input.tenant_id`
        // is the ONLY accepted case. A missing canonical tenant (`None` — e.g.
        // a direct `RegisterUserImpl::register` call that bypassed the
        // `#[tenant_scoped]` macro proxy, so `enforce_tenant` never ran) must
        // deny too, never fall through to using the client-controlled
        // `input.tenant_id` as the write-tenant unchecked.
        match ctx.canonical_tenant().and_then(|c| c.tenant_id()) {
            Some(resolved) if resolved.as_str() == input.tenant_id => {}
            _ => {
                return Err(RegisterUserError::Security(
                    SecurityError::AuthorizationDenied {
                        reason: format!(
                            "request tenant_id {:?} does not match a resolved authenticated tenant",
                            input.tenant_id
                        ),
                    },
                ));
            }
        }

        // AD-5: ensure the org FIRST (idempotent) — a User must never
        // reference a missing org.
        let org_ref = self
            .org_runtime
            .entity_ref::<TenantOrgCommand, TenantOrgState>(
                "tenant_organization",
                input.tenant_id.clone(),
                Arc::new(TenantOrganizationEntity::new()),
            )?;
        let org_result: CommandResult<OrganizationEnsured, TenantOrgState> = org_ref
            .send_command(
                TenantOrgCommand::Ensure {
                    org_id: input.tenant_id.clone(),
                    name: input.org_name.clone(),
                },
                // The identity the reservation accepted, carried down unchanged.
                // Both aggregates in this workflow get the same one, because
                // they are two steps of one business operation — that is what
                // lets the second step be recovered after the first already
                // completed. Read, never recomputed: deriving it again here
                // could differ from what the reservation used and turn a
                // legitimate retry into a permanent conflict.
                CommandContext::new("tenant_organization".to_string())
                    .carrying(ctx.operation_identity()),
            )
            .await?;
        match &org_result {
            CommandResult::Events { events, .. }
            | CommandResult::EffectsAcceptanceFailed { events, .. } => {
                self.publish_read_side(&input.tenant_id, events);
            }
            CommandResult::NoEvents { .. } => {}
            // This step already happened under this operation key. Its events
            // were projected when they were first written, so republishing
            // would duplicate read-side work for a command that did not run.
            //
            // The workflow continues to the user step regardless: that is the
            // whole point of a per-aggregate receipt after a partial failure —
            // the org is confirmed, the user may still be missing. Nothing here
            // needs data from the org step, so no current-state read is
            // required; if it ever did, that read would be explicit and would
            // not be presented as this command's historical answer.
            CommandResult::Replayed { .. } => {}
        }

        // Then register the User. On failure here, the org write above is
        // NOT rolled back (AD-5: no saga, no compensation — the org is left
        // as a benign, idempotently-reusable orphan).
        let user_ref = self.user_runtime.entity_ref::<UserCommand, UserState>(
            "user",
            input.user_id.clone(),
            Arc::new(UserEntity::new()),
        )?;
        let user_result: Result<CommandResult<UserRegistered, UserState>, EntityError> = user_ref
            .send_command(
                UserCommand::Register {
                    user_id: input.user_id.clone(),
                    email: input.email.clone(),
                    tenant_id: input.tenant_id.clone(),
                },
                // The same identity the org step above was given. Handing this
                // step a different one — or none — would make the two steps
                // belong to different operations, and a retry after a partial
                // failure would re-run this one instead of recovering it.
                CommandContext::new("user".to_string()).carrying(ctx.operation_identity()),
            )
            .await;

        match user_result {
            Ok(result) => {
                // AD-9: `EffectsAcceptanceFailed` is a real, committed write
                // with a post-commit warning attached, never a command
                // failure — the read-side projection must still learn about
                // the committed events exactly as it would for `Events`
                // (`UserEntity::external_effects` describes a
                // "welcome email" effect on every registration; this
                // reference app never wires an `EffectAcceptor`, so that
                // description always fails closed here — see
                // `domain/user.rs`).
                match &result {
                    CommandResult::Events { events, .. }
                    | CommandResult::EffectsAcceptanceFailed { events, .. } => {
                        self.publish_read_side(&input.tenant_id, events);
                    }
                    CommandResult::NoEvents { .. } => {}
                    // The user step already happened under this key. Same
                    // reasoning as the org branch: its events were projected
                    // when written. The response below is composed from the
                    // request, not from a replayed result, so a replay of this
                    // step is safe to fall through to success.
                    CommandResult::Replayed { .. } => {}
                }
                self.record("register_user.success", &input.user_id, "completed");
                Ok(RegisterOutput {
                    user_id: input.user_id,
                    tenant_id: input.tenant_id,
                })
            }
            Err(e) => {
                self.record(
                    "register_user.partial_failure",
                    &input.user_id,
                    "user_write_failed",
                );
                Err(RegisterUserError::from(e))
            }
        }
    }
}
