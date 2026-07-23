//! CORE-018 Phase 7 — non-atomic dual-write, partial-failure proof (AD-5).
//!
//! Satisfies reference-service spec "TenantOrganization succeeds, User write
//! fails": proves the org is left as a benign, idempotently-reusable orphan
//! (not just "org still exists") — a subsequent `Ensure` on the same org_id
//! must return zero new events.

mod support;

use std::sync::Arc;

use ego_testkit::{PrincipalBuilder, ServiceTestFixture};
use persistent_entity::command_context::CommandContext;
use persistent_entity::entity_ref::EntityRef;
use persistent_entity::persistent_entity::CommandResult;
use reference_app::application::{RegisterInput, RegisterUser, RegisterUserTag};
use reference_app::domain::tenant_org::{
    OrganizationEnsured, TenantOrgCommand, TenantOrgState, TenantOrganizationEntity,
};

fn input() -> RegisterInput {
    RegisterInput {
        user_id: "user-1".to_string(),
        // Empty email is UserEntity's real validation trigger (see
        // domain/user.rs) — drives a genuine User-write failure, not a
        // test-only backdoor.
        email: String::new(),
        tenant_id: "tenant-a".to_string(),
        org_name: "Acme".to_string(),
    }
}

#[tokio::test]
async fn user_write_failure_leaves_org_persisted_as_a_benign_reusable_orphan() {
    let (service, org_runtime) = support::make_register_user_full(None, None);

    let principal = PrincipalBuilder::new().tenant("tenant-a").build();
    let fixture = ServiceTestFixture::builder()
        .with_service::<RegisterUserTag>(service)
        .expect("registration succeeds")
        .principal(principal)
        .build();

    let proxy = fixture
        .resolve::<RegisterUserTag>()
        .expect("registered tag resolves");

    let ctx = fixture.context().with_tenant_id("tenant-a");
    let result = proxy.register(ctx, input()).await;

    assert!(
        result.is_err(),
        "RegisterUser must return Err when the User write fails after the org write succeeded"
    );

    // Prove the org persists (not rolled back) AND is a genuinely benign,
    // idempotently-reusable orphan: a subsequent Ensure on the same org_id
    // produces zero new events.
    let org_ref = org_runtime
        .entity_ref::<TenantOrgCommand, TenantOrgState>(
            "tenant_organization",
            "tenant-a".to_string(),
            Arc::new(TenantOrganizationEntity::new()),
        )
        .expect("entity_ref succeeds");
    let reensure: CommandResult<OrganizationEnsured, TenantOrgState> = org_ref
        .send_command(
            TenantOrgCommand::Ensure {
                org_id: "tenant-a".to_string(),
                name: "Acme".to_string(),
            },
            CommandContext::new("tenant_organization".to_string()),
        )
        .await
        .expect("re-ensure on the already-persisted org must succeed");

    match reensure {
        CommandResult::NoEvents { state } => {
            assert_eq!(
                state,
                TenantOrgState::Present {
                    name: "Acme".to_string()
                },
                "org must be Present (persisted from the first write)"
            );
        }
        CommandResult::Events { .. } => {
            panic!(
                "re-ensure produced new events — the org residue is not the benign, \
                 idempotently-reusable orphan AD-5 claims"
            );
        }
        // CORE-019 (not yet wired in this example, PR5 scope): this handler
        // never describes external effects, so this variant is unreachable
        // here — kept exhaustive rather than a wildcard so a future
        // regression that starts describing effects doesn't silently fall
        // through unnoticed.
        CommandResult::EffectsAcceptanceFailed { .. } => {
            panic!("this handler never describes external effects — unreachable");
        }
    }
}
