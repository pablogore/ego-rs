//! The runner that owns the run's PostgreSQL.
//!
//! ```console
//! cargo run --manifest-path integration-tests/Cargo.toml --bin run-suite
//! ```
//!
//! # Why a runner exists, and why it is not hypothetical complexity
//!
//! Issue #275 requires one shared PostgreSQL per run, migrations applied once, and
//! per-test isolation. Collapsing the eight test files into one target delivered
//! all three — and then could not clean up after itself.
//!
//! A test binary has no suite-level teardown. Holding the container in a
//! process-wide cell means its async `Drop` runs at process exit, when no Tokio
//! runtime is left to drive it: **three consecutive runs left three containers
//! behind**, where the old container-per-file shape had leaked none, because each
//! of those guards dropped inside a live runtime. testcontainers' `watchdog`
//! feature did not close it either — it handles signals, not ordinary exit.
//!
//! That is a concrete responsibility libtest cannot express, not a preference. So
//! it lives here: this process starts the container, prepares the template, runs
//! the suite as a child, and destroys the container **while its own runtime is
//! still alive**.
//!
//! # What it does not own
//!
//! Per-test isolation stays in the tests. Each one still clones its own database
//! from the template and closes its own pools; the runner never reaches into a
//! test's state. It provisions and reclaims, and nothing else.
//!
//! # The exit code is the suite's
//!
//! Propagated exactly, including on failure — a runner that swallowed a failing
//! suite would be worse than no runner. The container is destroyed on both paths
//! before that code is returned.

use std::process::ExitCode;

use sqlx::postgres::PgPoolOptions;
use sqlx::{Executor, PgPool};
use testcontainers::runners::AsyncRunner;
use testcontainers::ImageExt;
use testcontainers_modules::postgres::Postgres;

const TEMPLATE: &str = "ego_template";
const HOST_VAR: &str = "EGO_IT_PG_HOST";
const PORT_VAR: &str = "EGO_IT_PG_PORT";

async fn connect(url: &str) -> PgPool {
    PgPoolOptions::new()
        .max_connections(4)
        .connect(url)
        .await
        .expect("the container accepts connections")
}

/// Runs the hermetic ledger guard, before anything is provisioned.
///
/// It needs no container, so making it a precondition costs nothing and makes a
/// drifted ledger fail in milliseconds instead of after a container start, a
/// template clone and a full suite. It is also the one check here that can fail
/// for a reason no amount of PostgreSQL would reveal: a test file that exists,
/// is documented, and is registered nowhere runs never, and a suite that reports
/// success without it is the failure this guard exists to catch.
///
/// What the ledger preflight concluded.
///
/// Three outcomes, not two, because "the guard did not pass" and "the ledger
/// diverged" are different facts and only one of them is about the ledger.
/// Collapsing them was a real defect in the first version of this preflight: a
/// target that failed to *compile* also exits non-zero, and the runner reported
/// it as a documentation divergence that had not happened. A guard whose message
/// describes a case it does not cover is worse than no guard.
enum LedgerCheck {
    /// The guard ran and passed.
    Consistent,
    /// The guard ran and failed: the ledger, the module registration and the
    /// directory genuinely disagree.
    Diverged,
    /// The guard never got to answer — it could not be built, or cargo could not
    /// be started. This says nothing about the ledger.
    Unavailable(String),
}

/// Runs the hermetic ledger guard, before anything is provisioned.
///
/// It needs no container, so making it a precondition costs nothing and makes a
/// drifted ledger fail in milliseconds instead of after a container start, a
/// template clone and a full suite. It is also the one check here that can fail
/// for a reason no amount of PostgreSQL would reveal: a test file that exists,
/// is documented, and is registered nowhere runs never, and a suite that reports
/// success without it is the failure this guard exists to catch.
///
/// # Why two invocations
///
/// `cargo test` exits non-zero for a compile error and for a failing assertion
/// alike, so its status cannot tell those apart. `--no-run` builds without
/// running: if that fails, the answer is [`LedgerCheck::Unavailable`] and the
/// ledger was never inspected. Only once it builds does a non-zero status from
/// the run mean the sets actually disagree. The second invocation compiles
/// nothing new, so the cost is one cargo no-op.
fn check_ledger() -> LedgerCheck {
    const ARGS: [&str; 4] = [
        "--manifest-path",
        concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml"),
        "--test",
        "ledger",
    ];

    // Build only. A failure here is a toolchain or compilation fact, never a
    // statement about the ledger's contents.
    match std::process::Command::new(env!("CARGO"))
        .arg("test")
        .args(ARGS)
        .arg("--no-run")
        .status()
    {
        Ok(status) if status.success() => {}
        Ok(status) => {
            return LedgerCheck::Unavailable(format!(
                "the ledger guard could not be built (cargo test --no-run exited {status})"
            ))
        }
        // Same reasoning as the suite's own spawn below: a child that could not
        // be started at all is a failure to report, never a check to skip.
        Err(e) => return LedgerCheck::Unavailable(format!("cargo could not be started: {e}")),
    }

    // The guard reads three paths and panics if any is unreadable, so a sparse
    // checkout, a permission-restricted workspace or a partial clone would make
    // it exit non-zero for a reason that is not a divergence. Those are checked
    // here, where the answer can still be `Unavailable`.
    for path in [
        concat!(env!("CARGO_MANIFEST_DIR"), "/tests/infrastructure"),
        concat!(env!("CARGO_MANIFEST_DIR"), "/tests/infrastructure.rs"),
        concat!(env!("CARGO_MANIFEST_DIR"), "/README.md"),
    ] {
        if let Err(e) = std::fs::metadata(path) {
            return LedgerCheck::Unavailable(format!("{path} is not readable: {e}"));
        }
    }

    // It builds and its inputs are readable, so from here the guard's own
    // verdict is what a status means — but only for the status libtest uses to
    // report failing tests. A child killed by a signal reports no code at all
    // (an OOM kill, a `SIGTERM` from an impatient CI), and any other code came
    // from something that is not libtest reporting assertions.
    match std::process::Command::new(env!("CARGO"))
        .arg("test")
        .args(ARGS)
        .status()
    {
        Ok(status) if status.success() => LedgerCheck::Consistent,
        Ok(status) if status.code() == Some(101) => LedgerCheck::Diverged,
        Ok(status) => LedgerCheck::Unavailable(format!(
            "the ledger guard ended abnormally rather than reporting a verdict ({status})"
        )),
        Err(e) => LedgerCheck::Unavailable(format!("cargo could not be started: {e}")),
    }
}

#[tokio::main]
async fn main() -> ExitCode {
    let started = std::time::Instant::now();

    // Before the container: nothing below can make a drifted ledger correct, and
    // provisioning first would only make finding out slower. Each outcome names
    // what actually happened — a build failure is not a divergence.
    match check_ledger() {
        LedgerCheck::Consistent => {}
        LedgerCheck::Diverged => {
            eprintln!(
                "\n[integration-tests] the ledger and the suite disagree — \
                 nothing was provisioned. The guard's own output above names \
                 which side has the extra or missing entry."
            );
            return ExitCode::FAILURE;
        }
        LedgerCheck::Unavailable(why) => {
            eprintln!(
                "\n[integration-tests] the ledger guard did not run, so the ledger \
                 was never checked — nothing was provisioned. This is not a \
                 divergence: {why}"
            );
            return ExitCode::FAILURE;
        }
    }

    let container = Postgres::default()
        .with_tag("16")
        .start()
        .await
        .expect("a PostgreSQL container starts");
    let host = container.get_host().await.expect("a host").to_string();
    let port = container
        .get_host_port_ipv4(5432)
        .await
        .expect("the mapped port");
    let provisioned = started.elapsed();

    // The template is created and migrated exactly once, here. Both connections
    // are closed before the suite runs: PostgreSQL refuses
    // `CREATE DATABASE ... TEMPLATE t` while any session is connected to `t`, so
    // a pool left open would make every clone in every test fail.
    let admin = connect(&format!(
        "postgres://postgres:postgres@{host}:{port}/postgres"
    ))
    .await;
    admin
        .execute(format!("CREATE DATABASE {TEMPLATE}").as_str())
        .await
        .expect("the template database is created");
    admin.close().await;

    let template = connect(&format!(
        "postgres://postgres:postgres@{host}:{port}/{TEMPLATE}"
    ))
    .await;
    ego_persistence::postgres::migrations::run(&template)
        .await
        .expect("the real migrations apply to the template");
    template.close().await;
    let migrated = started.elapsed();

    // The suite, as a child process, told only where PostgreSQL is.
    //
    // `cargo test` rather than the compiled binary directly: the target may need
    // rebuilding, and reproducing cargo's target resolution here would be a
    // second place for it to drift.
    //
    // It costs, and the cost is measured rather than guessed: a freshness check
    // over this workspace's dependency graph is ~9.5s even with nothing to do, and
    // this run pays it twice — once for the outer `cargo run`, once here. Total
    // wall clock is ~22s against an 11.4s baseline, of which container start is
    // ~1.8s and the tests themselves ~2s. Well inside #275's five-minute budget,
    // and the obvious follow-up is to locate the built test binary via
    // `--message-format=json` and exec it, paying the check once.
    //
    // The result is kept, never unwrapped. An earlier version wrote
    // `.expect("the suite starts")`, which meant a child that could not be
    // spawned at all — a missing toolchain, an exhausted process table — panicked
    // straight past the reclamation below. The one path the runner exists to
    // guarantee was the one path that skipped it.
    let outcome = std::process::Command::new(env!("CARGO"))
        .args([
            "test",
            "--manifest-path",
            concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml"),
            "--test",
            "infrastructure",
        ])
        .env(HOST_VAR, &host)
        .env(PORT_VAR, port.to_string())
        .status();
    let ran = started.elapsed();

    // Destroyed here, inside a live runtime — the whole reason this process
    // exists. Reached on **every** path: the suite passing, the suite failing, and
    // the suite never starting. Awaited rather than left to `Drop` so a failure to
    // reclaim is visible rather than silent.
    let reclaimed = container.rm().await;

    // Visible and bounded, as #275 asks: the budget is a completion criterion,
    // so a run that does not report its cost cannot be checked against it.
    eprintln!(
        "\n[integration-tests] provisioned in {:.2}s · template migrated at {:.2}s · \
         suite finished at {:.2}s · reclaimed at {:.2}s",
        provisioned.as_secs_f64(),
        migrated.as_secs_f64(),
        ran.as_secs_f64(),
        started.elapsed().as_secs_f64(),
    );

    // Reclamation is reported before the suite's verdict, and never in place of
    // it: a container left behind is the runner's own failure, and hiding it
    // behind a green suite would make the invariant unobservable.
    if let Err(e) = reclaimed {
        eprintln!("[integration-tests] FAILED to reclaim the container: {e}");
        return ExitCode::FAILURE;
    }

    // Exactly the suite's outcome. `ExitCode::FAILURE` for a signal-terminated
    // child, which has no code of its own but is certainly not success, and for a
    // child that never started — reported here rather than panicked above, so the
    // container was reclaimed first.
    match outcome {
        Ok(status) => match status.code() {
            Some(0) => ExitCode::SUCCESS,
            Some(code) => ExitCode::from(u8::try_from(code).unwrap_or(1)),
            None => ExitCode::FAILURE,
        },
        Err(e) => {
            eprintln!("[integration-tests] the suite could not be started: {e}");
            ExitCode::FAILURE
        }
    }
}
