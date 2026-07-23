//! CORE-018 Phase 5 — `TenantOrganization` `PersistentEntity`, idempotent
//! ensure (AD-5, AD-6).
//!
//! Satisfies reference-service spec "Associating a user with a tenant org"
//! per this change's ground-truth resolution: AD-6 settled an idempotent
//! `Ensure`/`OrganizationEnsured`/`Absent|Present{name}` shape, not the
//! spec's stale `UserAssociatedWithTenant`/"membership set" wording (see
//! tasks.md ground-truth note, reconciled at sdd-verify).

use persistent_entity::command_context::CommandContext;
use persistent_entity::persistent_entity::PersistentEntity;
use reference_app::domain::tenant_org::{
    TenantOrgCommand, TenantOrgState, TenantOrganizationEntity,
};

fn ctx() -> CommandContext {
    CommandContext::new("tenant-org".to_string())
}

#[tokio::test]
async fn ensure_on_absent_produces_organization_ensured_and_transitions_to_present() {
    let entity = TenantOrganizationEntity::new();
    let state = entity.initial_state();
    assert_eq!(state, TenantOrgState::Absent);

    let cmd = TenantOrgCommand::Ensure {
        org_id: "org-1".to_string(),
        name: "Acme".to_string(),
    };

    let events = entity
        .handle_command(&cmd, &state, &ctx())
        .await
        .expect("ensure should succeed");
    assert_eq!(
        events.len(),
        1,
        "exactly one OrganizationEnsured event expected"
    );

    let new_state = entity
        .apply_event(&state, &events[0])
        .await
        .expect("apply_event should succeed");
    assert_eq!(
        new_state,
        TenantOrgState::Present {
            name: "Acme".to_string()
        }
    );
}

#[tokio::test]
async fn ensure_on_present_is_idempotent_and_produces_no_events() {
    let entity = TenantOrganizationEntity::new();
    let present = TenantOrgState::Present {
        name: "Acme".to_string(),
    };

    let cmd = TenantOrgCommand::Ensure {
        org_id: "org-1".to_string(),
        name: "Acme".to_string(),
    };

    let events = entity
        .handle_command(&cmd, &present, &ctx())
        .await
        .expect("re-ensure on an already-present org should not error");

    assert!(
        events.is_empty(),
        "handle_command must produce zero events on an already-present org \
         (this is what makes AD-5's 'benign reusable orphan' claim true — \
         the runtime/actor layer maps an empty event vec to CommandResult::NoEvents)"
    );
}
