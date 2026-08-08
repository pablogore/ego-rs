//! Readiness for the durable reservation store, against a real PostgreSQL.
//!
//! The unit tests in `ego-service-sdk` establish the contributor's mapping and its
//! wiring using a scripted double. They cannot establish the thing that actually
//! matters in production: that the durable store's `probe` really does answer `Ok`
//! against a reachable database, really does error against an unreachable one, and
//! that the two are distinguishable without a test controlling the answer. A double
//! that returns whatever it was told proves the contributor reads it correctly, not
//! that the adapter produces it correctly.
//!
//! # Losing Postgres without losing the container
//!
//! The outage is produced by a TCP forwarder the test owns, sitting between the pool
//! and the container, rather than by stopping the container. Two reasons, both
//! measured rather than assumed:
//!
//! - Docker re-allocates a dynamically published port on restart — a stopped and
//!   restarted container comes back on a *different* host port, so the pool's URL no
//!   longer points at it and the same store could never recover. Recovery is half of
//!   what this file is for.
//! - The forwarder's listener stays bound for the whole test, so the port cannot be
//!   taken by anything else during the outage window, and coming back up is a state
//!   change rather than a race to re-bind.
//!
//! Going down severs every established connection and refuses new ones immediately,
//! so the failure is a deterministic error rather than a timeout — no sleeping, and
//! no dependence on how fast the machine runs.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use chrono::{TimeZone, Utc};
use ego_domain::health::{HealthCode, HealthStatus, ProbeKind};
use ego_domain::operation::OperationReservationStore;
use ego_persistence::postgres::migrations;
use ego_persistence::postgres::reservation::PostgresOperationReservationStore;
use ego_service_sdk::health::{
    HealthAggregationConfig, HealthAggregator, HealthRegistry,
    OperationReservationStoreHealthContributor, OPERATION_RESERVATION_STORE_CONTRIBUTOR,
};
use ego_testkit::TestClock;
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use testcontainers::runners::AsyncRunner;
use testcontainers::{ContainerAsync, ImageExt};
use testcontainers_modules::postgres::Postgres;
use tokio::io::AsyncWriteExt;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::broadcast;

/// Pinned explicitly, matching the framework's declared PostgreSQL 14 floor.
const POSTGRES_IMAGE_TAG: &str = "14-alpine";

fn epoch() -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap()
}

/// The container's host address and mapped port.
async fn start_container() -> (ContainerAsync<Postgres>, String, u16) {
    let container = Postgres::default()
        .with_tag(POSTGRES_IMAGE_TAG)
        .start()
        .await
        .expect("the Postgres testcontainer must start; this test cannot run without Docker");

    let port = container
        .get_host_port_ipv4(5432)
        .await
        .expect("the container must publish its mapped Postgres port");
    let host = container
        .get_host()
        .await
        .expect("the container must report a reachable host address")
        .to_string();
    let host = if host == "localhost" {
        "127.0.0.1".to_string()
    } else {
        host
    };

    (container, host, port)
}

async fn connect(url: &str) -> PgPool {
    PgPoolOptions::new()
        .max_connections(4)
        // Bounded so a probe against a store that is down cannot outlive the test.
        // The forwarder refuses connections outright rather than blackholing them, so
        // this is a backstop, not the mechanism.
        .acquire_timeout(Duration::from_secs(5))
        .connect(url)
        .await
        .expect("must be able to connect through the forwarder")
}

/// A TCP forwarder that can be taken down and brought back up on the same port.
///
/// While up, it proxies bidirectionally to `target`. Taking it down severs every
/// connection it is currently proxying — which is what makes the pool's *established*
/// connections fail, not just its next new one — and makes subsequent connections
/// close immediately on accept. The listener itself is never released, so the port
/// stays reserved for the whole test.
struct Forwarder {
    port: u16,
    up: Arc<AtomicBool>,
    sever: broadcast::Sender<()>,
}

impl Forwarder {
    async fn start(target: String) -> Self {
        // Port 0: the OS picks a free one, so this can never collide with another
        // test or with something already running on the machine.
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("binding an ephemeral forwarder port must succeed");
        let port = listener
            .local_addr()
            .expect("the bound listener must report its address")
            .port();
        let up = Arc::new(AtomicBool::new(true));
        let (sever, _) = broadcast::channel(16);

        let accept_up = Arc::clone(&up);
        let accept_sever = sever.clone();
        tokio::spawn(async move {
            loop {
                let Ok((mut inbound, _)) = listener.accept().await else {
                    return;
                };
                if !accept_up.load(Ordering::SeqCst) {
                    // Down: accept and drop. The client sees the connection close
                    // immediately, which is a deterministic error rather than a
                    // hang the test would have to time out on.
                    let _ = inbound.shutdown().await;
                    continue;
                }
                let target = target.clone();
                let mut severed = accept_sever.subscribe();
                tokio::spawn(async move {
                    let Ok(mut outbound) = TcpStream::connect(&target).await else {
                        return;
                    };
                    tokio::select! {
                        _ = tokio::io::copy_bidirectional(&mut inbound, &mut outbound) => {}
                        // The store going down drops both halves here, so a
                        // connection the pool already holds breaks too.
                        _ = severed.recv() => {}
                    }
                });
            }
        });

        Self { port, up, sever }
    }

    fn url(&self) -> String {
        format!(
            "postgres://postgres:postgres@127.0.0.1:{}/postgres",
            self.port
        )
    }

    fn take_down(&self) {
        self.up.store(false, Ordering::SeqCst);
        // Ignored: with no proxied connection in flight there is no receiver, which
        // is not a failure.
        let _ = self.sever.send(());
    }

    fn bring_up(&self) {
        self.up.store(true, Ordering::SeqCst);
    }
}

/// One aggregator over one contributor over `store` — the real fold, not a direct
/// `check()` call, so the report a probe endpoint would serve is what gets asserted.
fn contributor(store: PostgresOperationReservationStore) -> Arc<HealthAggregator> {
    let contributor = OperationReservationStoreHealthContributor::new(
        Arc::new(store) as Arc<dyn OperationReservationStore>
    );
    Arc::new(HealthAggregator::new(
        HealthRegistry::from_contributors(vec![Arc::new(contributor)]),
        HealthAggregationConfig {
            // Well under the pool's own acquire timeout, so a store that hangs
            // rather than refusing still lands as `Timeout` inside the test's
            // lifetime instead of stalling it.
            per_contributor: Duration::from_secs(3),
            global_budget: None,
        },
    ))
}

/// The durable store's probe answers `Ok` against a reachable, migrated database,
/// and the fold says ready.
#[tokio::test(flavor = "multi_thread")]
async fn a_reachable_store_reports_ready() {
    let (_container, host, port) = start_container().await;
    let url = format!("postgres://postgres:postgres@{host}:{port}/postgres");
    let pool = connect(&url).await;
    migrations::run(&pool)
        .await
        .expect("the framework's own migrations must apply cleanly");

    let aggregator = contributor(PostgresOperationReservationStore::new(
        pool.clone(),
        Arc::new(TestClock::new(epoch())),
    ));

    let report = aggregator.readiness().await;

    assert_eq!(report.probe, ProbeKind::Readiness);
    assert_eq!(report.status, HealthStatus::Healthy);
    let store_report = report
        .contributors
        .iter()
        .find(|c| c.name == OPERATION_RESERVATION_STORE_CONTRIBUTOR)
        .expect("the store must be reported on");
    assert_eq!(store_report.code, None);
}

/// An empty table is a reachable store.
///
/// The probe reads a row and discards it; a table with nothing in it must not be
/// mistaken for a database that cannot be talked to. Every deployment is in this
/// state immediately after its first migration, so getting this wrong would make a
/// fresh install permanently un-ready.
#[tokio::test(flavor = "multi_thread")]
async fn an_empty_table_is_still_a_reachable_store() {
    let (_container, host, port) = start_container().await;
    let url = format!("postgres://postgres:postgres@{host}:{port}/postgres");
    let pool = connect(&url).await;
    migrations::run(&pool)
        .await
        .expect("migrations must apply cleanly");

    let rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM operation_reservations")
        .fetch_one(&pool)
        .await
        .expect("counting must succeed");
    assert_eq!(rows, 0, "this test's premise is a freshly migrated table");

    let store =
        PostgresOperationReservationStore::new(pool.clone(), Arc::new(TestClock::new(epoch())));

    assert_eq!(store.probe().await, Ok(()));
}

/// The probe never creates, mutates or purges anything.
///
/// A health check that writes makes the act of asking change the thing being asked
/// about, and this one runs on every readiness poll — at that frequency a probe that
/// reserved a key would fill the table on its own. Asserting the row count is
/// unchanged is the direct statement of that; the store is otherwise exercised
/// normally around it so the count is not trivially zero on both sides.
#[tokio::test(flavor = "multi_thread")]
async fn probing_leaves_the_table_exactly_as_it_was() {
    use chrono::Duration as ChronoDuration;
    use ego_domain::operation::{OperationFingerprint, OperationKey, OwnerId, ReserveRequest};
    use ego_domain::Clock;

    let (_container, host, port) = start_container().await;
    let url = format!("postgres://postgres:postgres@{host}:{port}/postgres");
    let pool = connect(&url).await;
    migrations::run(&pool)
        .await
        .expect("migrations must apply cleanly");

    let clock = Arc::new(TestClock::new(epoch()));
    let store = PostgresOperationReservationStore::new(pool.clone(), clock.clone());
    store
        .reserve(ReserveRequest {
            tenant: None,
            operation_key: OperationKey::parse("op-probe-is-read-only").expect("valid key"),
            fingerprint: OperationFingerprint::new("fp-1"),
            owner_id: OwnerId::new("owner"),
            lease_until: clock.now() + ChronoDuration::seconds(30),
        })
        .await
        .expect("the reservation must succeed");

    let before: Vec<(String, String, i64, chrono::DateTime<Utc>)> = sqlx::query_as(
        "SELECT operation_key, owner_id, fencing_token, lease_until \
         FROM operation_reservations ORDER BY operation_key",
    )
    .fetch_all(&pool)
    .await
    .expect("reading the table must succeed");

    for _ in 0..20 {
        store.probe().await.expect("the store is reachable");
    }

    let after: Vec<(String, String, i64, chrono::DateTime<Utc>)> = sqlx::query_as(
        "SELECT operation_key, owner_id, fencing_token, lease_until \
         FROM operation_reservations ORDER BY operation_key",
    )
    .fetch_all(&pool)
    .await
    .expect("reading the table must succeed");

    assert_eq!(
        before, after,
        "twenty readiness probes must leave every row byte-identical: a probe that \
         reserved, renewed or purged anything would change what it is reporting on"
    );
}

/// Healthy, then unreachable, then healthy again — the same store instance
/// throughout.
///
/// Recovery is the half that cannot be skipped. Every other assertion here is
/// satisfied by a contributor that latches its first failure forever, and such a
/// contributor would leave an instance permanently out of rotation after an outage
/// that has already ended. This is also why losing Postgres must not take the process
/// down: the recovery below happens without a restart, and a supervisor that killed
/// the process on an unhealthy readiness would have replaced it with a new one facing
/// the same unreachable database, in a loop.
#[tokio::test(flavor = "multi_thread")]
async fn readiness_follows_postgres_down_and_back_up() {
    let (_container, host, port) = start_container().await;
    let forwarder = Forwarder::start(format!("{host}:{port}")).await;
    let pool = connect(&forwarder.url()).await;
    migrations::run(&pool)
        .await
        .expect("migrations must apply cleanly through the forwarder");

    let aggregator = contributor(PostgresOperationReservationStore::new(
        pool.clone(),
        Arc::new(TestClock::new(epoch())),
    ));

    // (1) Reachable.
    assert_eq!(
        aggregator.readiness().await.status,
        HealthStatus::Healthy,
        "the premise is a store that starts out reachable"
    );

    // (2) Postgres becomes unreachable. Established connections are severed, so
    //     the pool cannot answer from one it already holds.
    forwarder.take_down();
    let down = aggregator.readiness().await;
    assert_eq!(
        down.status,
        HealthStatus::Unhealthy,
        "an unreachable store must take the instance out of rotation"
    );
    let down_report = down
        .contributors
        .iter()
        .find(|c| c.name == OPERATION_RESERVATION_STORE_CONTRIBUTOR)
        .expect("the store must still be reported on while it is down");
    assert!(
        matches!(
            down_report.code,
            Some(HealthCode::Unavailable) | Some(HealthCode::Timeout)
        ),
        "the report must say the dependency could not be reached, got {:?}",
        down_report.code
    );
    let rendered = format!("{down:?}");
    assert!(
        !rendered.contains("postgres://") && !rendered.contains("password"),
        "an unauthenticated readiness payload must not carry connection detail: {rendered}"
    );

    // (3) Postgres comes back. No rebuild, no restart, no new store.
    forwarder.bring_up();
    let recovered = aggregator.readiness().await;
    assert_eq!(
        recovered.status,
        HealthStatus::Healthy,
        "readiness must recover on its own once the store is reachable again — the \
         process was never the thing that was broken"
    );
    assert_eq!(
        recovered
            .contributors
            .iter()
            .find(|c| c.name == OPERATION_RESERVATION_STORE_CONTRIBUTOR)
            .expect("the contributor is still registered")
            .code,
        None,
        "a recovered contributor must carry no failure code"
    );
}
