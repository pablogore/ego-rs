//! PROD-013 AD-8/AD-9: `EntityEventStores::open(pool)` declares
//! `Profile::Production` **and** backs both aggregates' snapshot stores with
//! a real `PostgreSQLSnapshotStore` over the same pool — closing the
//! defect AD-9 describes: the reference app's production path wrote events
//! to Postgres and snapshots to process memory, silently.
//!
//! **Layers traversed:** `EntityEventStores::open` → `PostgreSQLSnapshotStore`
//! → real SQL against a real PostgreSQL with real migrations (the
//! `snapshots` table).
//!
//! # Why the second test cannot be a type check
//!
//! `org_snapshot`/`user_snapshot` are `Arc<Mutex<dyn Snapshot + Send>>` —
//! the same field type an `InMemorySnapshotStore` behind it would also
//! satisfy. Only a snapshot that survives the *pool* it was written through
//! outliving the *process* that wrote it distinguishes a durable store from
//! a volatile one wearing the same trait object.
//!
//! Run: `cargo run --manifest-path integration-tests/Cargo.toml --bin run-suite`.
//! Never `cargo test --workspace` at the root — this workspace is not a member.

use ego_integration_tests::isolated_database;
use ego_service_sdk::runtime::Profile;
use reference_app::EntityEventStores;
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;

async fn connect(url: &str) -> PgPool {
    PgPoolOptions::new()
        .max_connections(4)
        .connect(url)
        .await
        .expect("the container accepts connections")
}

/// `open()` is the only thing that declares `Profile::Production` — the
/// reference app's own production composition proves it, not a unit test
/// against a stub pool.
#[tokio::test]
async fn opened_stores_declare_profile_production() {
    let db = isolated_database().await;
    let pool = connect(db.url()).await;

    let stores = EntityEventStores::open(pool)
        .await
        .expect("the stores open against a migrated database");

    assert_eq!(stores.profile(), Profile::Production);

    db.close().await;
}

/// A snapshot written through one `open()` instance survives a fresh
/// `open()` against the same pool, once the process that wrote it is gone.
#[tokio::test]
async fn a_written_snapshot_survives_a_fresh_open_against_the_same_pool() {
    let db = isolated_database().await;
    let url = db.url().to_string();

    {
        let stores = EntityEventStores::open(connect(&url).await)
            .await
            .expect("the stores open");
        stores
            .org_snapshot
            .lock()
            .save_snapshot("org-1", None, 3, serde_json::json!({"name": "Acme"}))
            .expect("the snapshot saves");
        // `stores`, and the pool it holds, are dropped here. Nothing left
        // in this process can be what the read below finds.
    }

    let stores = EntityEventStores::open(connect(&url).await)
        .await
        .expect("the stores open again");
    let loaded = stores
        .org_snapshot
        .lock()
        .load_snapshot("org-1", None)
        .expect("the snapshot loads");

    assert_eq!(
        loaded,
        Some((3, serde_json::json!({"name": "Acme"}))),
        "a fresh `EntityEventStores::open` over the same pool must see the \
         snapshot a previous, now-dropped instance wrote — nothing here \
         shares process memory, so this can only be a real durable store"
    );

    db.close().await;
}
