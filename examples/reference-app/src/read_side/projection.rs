//! `UsersByTenant` read model + its CORE-005 `Handler` (design.md's reference
//! journey, read side): for a given tenant, the set of registered users
//! plus the tenant organization's name.
//!
//! This is a real `ego_domain::read_side::handler::Handler` implementation
//! — it is invoked by CORE-005's real batch-processing engine
//! (`ego-runtime`'s `TagSchedulerImpl` -> `ReadSideSession`), never called
//! directly by application code. See `super` for the wiring.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use async_trait::async_trait;
use ego_domain::read_side::error::ProjectionError;
use ego_domain::read_side::event_stream::EventStreamElement;
use ego_domain::read_side::handler::Handler;
use serde::Serialize;
use serde_json::Value;
use utoipa::ToSchema;

/// One registered user, as seen by the `UsersByTenant` projection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct UserSummary {
    pub user_id: String,
    pub email: String,
}

/// The `UsersByTenant` read model for a single tenant: its organization
/// name (once `OrganizationEnsured` has been observed) plus every user
/// registered under it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, ToSchema)]
pub struct TenantUsersView {
    pub org_name: Option<String>,
    pub users: Vec<UserSummary>,
}

/// Shared, queryable handle to the `UsersByTenant` read model. Cheap to
/// clone (`Arc` inside) — one instance is shared between the HTTP query
/// route and the `Handler` that populates it.
#[derive(Clone, Default)]
pub struct UsersByTenantStore(Arc<RwLock<HashMap<String, TenantUsersView>>>);

impl UsersByTenantStore {
    /// Returns the current view for a tenant (empty default if nothing has
    /// been projected for it yet).
    pub fn view(&self, tenant_id: &str) -> TenantUsersView {
        self.0.read().expect("UsersByTenantStore lock poisoned").get(tenant_id).cloned().unwrap_or_default()
    }
}

/// CORE-005 `Handler` that projects `OrganizationEnsured`/`UserRegistered`
/// events (routed through the real read-side engine, see `super::spawn`)
/// into `UsersByTenantStore`.
#[derive(Clone)]
pub struct UsersByTenantHandler {
    store: UsersByTenantStore,
}

impl UsersByTenantHandler {
    pub fn new(store: UsersByTenantStore) -> Self {
        Self { store }
    }
}

#[async_trait]
impl Handler<Value> for UsersByTenantHandler {
    async fn handle(&self, events: &[EventStreamElement<Value>]) -> Result<(), ProjectionError> {
        let mut guard = self.store.0.write().expect("UsersByTenantStore lock poisoned");

        for event in events {
            match event.event_type() {
                "UserRegistered" => {
                    let email = event
                        .payload
                        .get("email")
                        .and_then(Value::as_str)
                        .ok_or_else(|| ProjectionError::poison_event("UserRegistered event missing email in payload"))?
                        .to_string();
                    guard.entry(event.tenant_id().to_string()).or_default().users.push(UserSummary {
                        user_id: event.aggregate_id().to_string(),
                        email,
                    });
                }
                "OrganizationEnsured" => {
                    let name = event
                        .payload
                        .get("name")
                        .and_then(Value::as_str)
                        .ok_or_else(|| ProjectionError::poison_event("OrganizationEnsured event missing name in payload"))?
                        .to_string();
                    guard.entry(event.tenant_id().to_string()).or_default().org_name = Some(name);
                }
                other => {
                    return Err(ProjectionError::poison_event(format!(
                        "UsersByTenant: unrecognized event_type {other}"
                    )))
                }
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use ego_domain::read_side::event_tag::EventTag;

    use super::*;

    fn user_registered(tenant_id: &str, user_id: &str, email: &str, version: i64) -> EventStreamElement<Value> {
        EventStreamElement::new(
            format!("UserRegistered:{user_id}:{version}"),
            user_id,
            tenant_id,
            "UserRegistered",
            serde_json::json!({ "email": email }),
            version,
            Utc::now(),
            vec![EventTag::new("users-by-tenant")],
        )
    }

    fn organization_ensured(tenant_id: &str, name: &str, version: i64) -> EventStreamElement<Value> {
        EventStreamElement::new(
            format!("OrganizationEnsured:{tenant_id}:{version}"),
            tenant_id,
            tenant_id,
            "OrganizationEnsured",
            serde_json::json!({ "name": name }),
            version,
            Utc::now(),
            vec![EventTag::new("users-by-tenant")],
        )
    }

    #[tokio::test]
    async fn handle_projects_org_and_user_events_into_the_tenant_view() {
        let store = UsersByTenantStore::default();
        let handler = UsersByTenantHandler::new(store.clone());

        handler
            .handle(&[organization_ensured("tenant-a", "Acme", 1), user_registered("tenant-a", "user-1", "u@e.com", 2)])
            .await
            .expect("handle succeeds");

        let view = store.view("tenant-a");
        assert_eq!(view.org_name.as_deref(), Some("Acme"));
        assert_eq!(view.users, vec![UserSummary { user_id: "user-1".to_string(), email: "u@e.com".to_string() }]);
    }

    #[tokio::test]
    async fn handle_keeps_tenants_isolated() {
        let store = UsersByTenantStore::default();
        let handler = UsersByTenantHandler::new(store.clone());

        handler.handle(&[user_registered("tenant-a", "user-1", "a@e.com", 1)]).await.unwrap();
        handler.handle(&[user_registered("tenant-b", "user-2", "b@e.com", 1)]).await.unwrap();

        assert_eq!(store.view("tenant-a").users.len(), 1);
        assert_eq!(store.view("tenant-b").users.len(), 1);
        assert!(store.view("tenant-c").users.is_empty());
    }

    #[tokio::test]
    async fn handle_rejects_an_unrecognized_event_type_as_a_poison_event() {
        let store = UsersByTenantStore::default();
        let handler = UsersByTenantHandler::new(store.clone());

        let unknown = EventStreamElement::new(
            "evt-1",
            "agg-1",
            "tenant-a",
            "SomethingElse",
            serde_json::json!({}),
            1,
            Utc::now(),
            vec![EventTag::new("users-by-tenant")],
        );

        let err = handler.handle(&[unknown]).await.unwrap_err();
        assert!(err.is_poison_event());
    }
}
