//! Shared fixtures for the infrastructure-backed suite.
//!
//! See `README.md` for the admission rules that govern what may live here. In
//! short: a scenario is admitted only if it traverses a capability end to end
//! and could fail in a way no in-process test can detect.
//!
//! This crate is a library so the fixtures — a shared PostgreSQL, migrations
//! applied once per run, per-test isolation — are defined once and reused,
//! rather than rebuilt per test file the way ad-hoc harnesses tend to be.
//!
//! # What this module is, and what it replaced
//!
//! The paragraph above described the intended shape from the start; the body was
//! empty, and each test file started its own container and ran the migrations
//! again. Eight files, eight containers, eight migration runs — which issue #275
//! forbids in as many words: *"One shared PostgreSQL per run, isolated per test
//! by schema or by database. Not one container per test."*
//!
//! **This is not a speed fix, and it is worth being exact about that.** Measured
//! warm, the whole suite ran in 11.4–12.0s with eight containers; container start
//! and migrations cost roughly 0.2–0.7s each, not the several seconds a single
//! cold measurement had suggested. The suite was already far inside #275's
//! five-minute budget. What the old shape actually cost was compliance and
//! linearity: every new test file added another container, and the next one
//! would have been the ninth.
//!
//! # The mechanism
//!
//! One container per run. One **template** database, migrated exactly once. Each
//! test gets its **own database**, cloned from that template — so a test sees a
//! fully-migrated, completely empty schema that no other test can reach.
//!
//! Isolation by database rather than by schema, deliberately. Several tests in
//! this suite inspect whole tables — `SELECT operation_key FROM
//! operation_reservations` with no `WHERE`, for instance — and within its own
//! database that query is *correct*: scanning everything is the point. Schema
//! isolation would have forced a `search_path` discipline on every one of them,
//! and rewriting a query to survive its harness is how a test stops meaning what
//! it says.
//!
//! # Concurrency
//!
//! A semaphore bounds how many isolated databases are live at once, because the
//! container's connection budget is now shared where it used to be one container
//! per file. It is there to protect connections, **not** to serialise the suite:
//! the permit is held for the life of the database, and the limit is well above
//! one.
//!
//! # Two things libtest cannot express, both learned by measuring
//!
//! **A shared `sqlx` pool cannot cross tests.** Every `#[tokio::test]` builds its
//! own runtime and drops it when the test ends, and a pool holds background tasks
//! on the runtime that created it. A pool parked in a process-wide cell belongs to
//! whichever test ran first and dies with it. Sharing an admin pool produced
//! `PoolTimedOut` and `A Tokio 1.x context was found, but it is being shutdown`
//! across six of fifteen tests, and pushed the suite from 11.4s to 124s.
//!
//! So nothing pooled is shared. `CREATE DATABASE` runs over a single short-lived
//! connection opened and closed inside the calling test's own runtime, and the
//! serialising mutex is taken **before** that connection is opened — a connection
//! held while queueing for a lock is a connection nobody else can use, which is
//! exactly how the first attempt starved itself.
//!
//! **A test binary cannot own the container either.** Holding it in a
//! process-wide cell means its async `Drop` runs at process exit, when no runtime
//! is left to drive it: three consecutive runs left three containers behind, where
//! the old container-per-file shape had leaked none. Enabling testcontainers'
//! `watchdog` feature did not help — it handles signals, not ordinary exit.
//!
//! libtest simply has no suite-level teardown to hang that on. So the container
//! and the template belong to the `run-suite` binary — a separate target, which is
//! why it cannot be linked from here — and it destroys them while its own runtime
//! is still alive. This module only reads the address that runner published and
//! clones databases from the template it prepared.

use std::sync::Arc;

use sqlx::postgres::PgPoolOptions;
use sqlx::{Executor, PgPool};
use tokio::sync::{Mutex, OnceCell, OwnedSemaphorePermit, Semaphore};

/// The database every per-test database is cloned from. Created and migrated by
/// the runner, never by a test.
const TEMPLATE: &str = "ego_template";

/// Where the runner published the PostgreSQL it owns.
const HOST_VAR: &str = "EGO_IT_PG_HOST";
const PORT_VAR: &str = "EGO_IT_PG_PORT";

/// How many isolated databases may be live at once.
///
/// Bounds connections without serialising: the container's default
/// `max_connections` is 100 and each test opens a small pool, so this leaves
/// headroom while keeping the suite genuinely concurrent. A limit of one would
/// satisfy the budget and defeat the purpose.
const MAX_LIVE_DATABASES: usize = 8;

/// The address of the run's PostgreSQL, and the concurrency budget.
///
/// Holds no pool and no connection, deliberately — see the module docs.
struct Shared {
    host: String,
    port: u16,
    /// Serialises `CREATE DATABASE`, which cannot run concurrently against one
    /// template.
    creating: Mutex<()>,
    budget: Arc<Semaphore>,
    next: std::sync::atomic::AtomicU64,
}

static SHARED: OnceCell<Arc<Shared>> = OnceCell::const_new();

async fn shared() -> Arc<Shared> {
    SHARED
        .get_or_init(|| async {
            let host = std::env::var(HOST_VAR).unwrap_or_else(|_| missing_runner(HOST_VAR));
            let port: u16 = std::env::var(PORT_VAR)
                .unwrap_or_else(|_| missing_runner(PORT_VAR))
                .parse()
                .expect("the runner publishes a numeric port");

            Arc::new(Shared {
                host,
                port,
                creating: Mutex::new(()),
                budget: Arc::new(Semaphore::new(MAX_LIVE_DATABASES)),
                next: std::sync::atomic::AtomicU64::new(0),
            })
        })
        .await
        .clone()
}

/// Fails with the command that would have worked.
///
/// Reached when the tests are run directly instead of through the runner. Worth a
/// real message rather than a missing-variable panic: `cargo test --manifest-path
/// integration-tests/Cargo.toml` was the documented command for a long time, and
/// it now produces no PostgreSQL at all.
fn missing_runner(var: &str) -> ! {
    panic!(
        "{var} is not set, so no PostgreSQL was provisioned for this run.\n\n\
         This suite is started by its runner, which owns the container's whole \
         lifecycle:\n\n    \
         cargo run --manifest-path integration-tests/Cargo.toml --bin run-suite\n\n\
         Running the test target directly skips the runner, and with it the \
         container, the template database and the migrations."
    )
}

fn url_for(host: &str, port: u16, database: &str) -> String {
    format!("postgres://postgres:postgres@{host}:{port}/{database}")
}

async fn connect(url: &str) -> PgPool {
    PgPoolOptions::new()
        .max_connections(4)
        .connect(url)
        .await
        .expect("the runner's PostgreSQL accepts connections")
}

/// One test's own database: fully migrated, completely empty, unreachable by any
/// other test.
///
/// Cloned from the template rather than migrated again, so the migrations run
/// once per run no matter how many tests there are.
pub struct IsolatedDatabase {
    shared: Arc<Shared>,
    name: String,
    url: String,
    /// Every pool this guard handed out.
    ///
    /// Tracked so closing is structural rather than a discipline each test has to
    /// remember. A test that had to close three differently-named pools before
    /// dropping its database would eventually forget one, and the failure — a
    /// `DROP` refused by a lingering session — would surface far from its cause.
    pools: Mutex<Vec<PgPool>>,
    /// Released when this database is dropped, which is what keeps the
    /// connection budget honest.
    _permit: OwnedSemaphorePermit,
}

impl IsolatedDatabase {
    /// The connection URL for this database.
    ///
    /// Exposed because several tests open more than one pool against the same
    /// database on purpose — a store's pool and the test's own inspection pool,
    /// so the store can never be starved by the test.
    pub fn url(&self) -> &str {
        &self.url
    }

    /// A fresh pool against this database, owned by this guard.
    ///
    /// [`Self::close`] closes every pool taken from here, so a caller does not
    /// have to track them. A test that opens a pool by other means still owns it
    /// and must close it itself — `close()`'s `WITH (FORCE)` will terminate the
    /// session either way, but a pool nobody closed is a pool nobody accounted
    /// for.
    pub async fn pool(&self) -> PgPool {
        let pool = connect(&self.url).await;
        self.pools.lock().await.push(pool.clone());
        pool
    }

    /// Drops the database, releasing its share of the connection budget.
    ///
    /// Explicit rather than left to `Drop`, because dropping a database requires
    /// awaiting and no session may be connected to it. **Every test must call
    /// this**: skipping it leaves the database alive until the runner destroys the
    /// container, which makes the semaphore's count a fiction — it would be
    /// bounding permits taken rather than databases live, and the suite would grow
    /// internally for the whole run.
    ///
    /// # It panics rather than ignoring a failure
    ///
    /// An earlier version discarded the result. That made this method look like
    /// cleanup while guaranteeing none: a `DROP` refused because a caller had left
    /// a pool open would pass silently, and the leak it was written to prevent
    /// would happen anyway with a call site that appeared to prevent it.
    ///
    /// `WITH (FORCE)` terminates any session still attached, so the usual reason a
    /// drop fails is handled rather than tolerated — but a caller should still
    /// close its own pools first, and a failure here now says so out loud.
    pub async fn close(self) {
        // The pools first, then the database. A `DROP` issued while this guard's
        // own sessions are still open would rely on `FORCE` to terminate
        // connections the guard could simply have closed.
        for pool in self.pools.lock().await.drain(..) {
            pool.close().await;
        }

        let admin = connect(&url_for(&self.shared.host, self.shared.port, "postgres")).await;
        let dropped = admin
            .execute(format!("DROP DATABASE IF EXISTS {} WITH (FORCE)", self.name).as_str())
            .await;
        admin.close().await;
        dropped.unwrap_or_else(|e| {
            panic!(
                "the isolated database {} could not be dropped: {e}\n\n\
                 Every test must close its own pools before calling `close()`. A \
                 database left behind stays alive until the runner destroys the \
                 container, and the semaphore stops meaning what it says.",
                self.name
            )
        });
    }
}

/// Starts the run's PostgreSQL if it is not running, and hands back a database
/// no other test shares.
///
/// The container, the template and the migrations happen on the first call and
/// never again. Every later call pays only for a clone.
pub async fn isolated_database() -> IsolatedDatabase {
    let shared = shared().await;
    let permit = shared
        .budget
        .clone()
        .acquire_owned()
        .await
        .expect("the budget semaphore is never closed");

    let n = shared
        .next
        .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let name = format!("ego_test_{n}");

    {
        // The lock first, the connection second. PostgreSQL cannot clone one
        // template concurrently, so the create is serialised — and a connection
        // held while queueing for that lock is a connection nobody else can use,
        // which is precisely how the first version of this starved itself.
        let _creating = shared.creating.lock().await;
        let admin = connect(&url_for(&shared.host, shared.port, "postgres")).await;
        admin
            .execute(format!("CREATE DATABASE {name} TEMPLATE {TEMPLATE}").as_str())
            .await
            .expect("an isolated database is cloned from the migrated template");
        admin.close().await;
    }

    let url = url_for(&shared.host, shared.port, &name);
    IsolatedDatabase {
        shared,
        name,
        url,
        pools: Mutex::new(Vec::new()),
        _permit: permit,
    }
}
