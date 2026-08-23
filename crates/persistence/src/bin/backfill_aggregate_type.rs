//! Runbook entry point for the `events.aggregate_type` backfill.
//!
//! Applies the framework's own migrations (idempotent — safe to run against
//! an already-migrated database) and then runs the backfill against the
//! registered entity types supplied on the command line. Prints the
//! machine-readable report as JSON on stdout and exits non-zero if the
//! backfill was aborted or could not run at all, so a calling pipeline can
//! branch on the exit code without parsing the report.
//!
//! Usage:
//!
//! ```text
//! DATABASE_URL=postgres://user:pass@host:5432/dbname \
//!   backfill_aggregate_type <registered-entity-type> [<registered-entity-type> ...]
//! ```
//!
//! # Order of operations, and why it is not optional
//!
//! ```text
//! 1. quiesce the old writers   — stop every process still writing untyped rows
//! 2. run this tool             — applies the migration, then the backfill
//! 3. read the report           — a non-zero exit means nothing was consolidated
//! 4. the tool has already set the column mandatory on success
//! 5. start the new binary      — only now
//! ```
//!
//! Step 1 is the one most easily skipped and the most damaging to skip. While an
//! old instance is still inserting rows without a type, this tool can commit a
//! table that is complete at the instant it checked and incomplete a moment
//! later. The store's own open-time check will then refuse to start the new
//! binary — which is the intended outcome, but it turns a clean transition into
//! an outage to diagnose.
//!
//! Step 5 last, for the reason the store refuses to open otherwise: every read
//! and the version check filter on the type column, so against an untyped row
//! neither filter matches. Such a stream reads as absent, the version check
//! returns zero, and an append writes a second forked stream while the original
//! rows sit orphaned. Running the new binary first does not degrade reads; it
//! diverges history, and there is no clean recovery once traffic has passed
//! through.
//!
//! The open-time check exists precisely because this ordering cannot be enforced
//! by a document. It does not replace the order — it makes getting the order
//! wrong visible and recoverable instead of silent.

use ego_persistence::postgres::aggregate_type_backfill::{self, BackfillOutcome};
use ego_persistence::postgres::migrations;
use sqlx::postgres::PgPoolOptions;

#[tokio::main]
async fn main() {
    let registered_types: Vec<String> = std::env::args().skip(1).collect();
    if registered_types.is_empty() {
        eprintln!(
            "usage: backfill_aggregate_type <registered-entity-type> [<registered-entity-type> ...]"
        );
        std::process::exit(2);
    }

    let database_url = match std::env::var("DATABASE_URL") {
        Ok(url) => url,
        Err(_) => {
            eprintln!("DATABASE_URL must be set to the target Postgres instance");
            std::process::exit(2);
        }
    };

    let pool = match PgPoolOptions::new().connect(&database_url).await {
        Ok(pool) => pool,
        Err(err) => {
            eprintln!("failed to connect to the target database: {err}");
            std::process::exit(1);
        }
    };

    if let Err(err) = migrations::run(&pool).await {
        eprintln!("framework migrations failed to apply: {err}");
        std::process::exit(1);
    }

    match aggregate_type_backfill::backfill_aggregate_type(&pool, &registered_types).await {
        Ok(report) => {
            println!(
                "{}",
                serde_json::to_string_pretty(&report).expect("report is always serializable")
            );
            match report.outcome {
                BackfillOutcome::Committed => std::process::exit(0),
                // Both refusals share one exit code: what a pipeline has to
                // decide is whether the transition consolidated, and it did
                // not in either case. Which of the two it was, and why, is in
                // the report on stdout.
                BackfillOutcome::Aborted(_) | BackfillOutcome::RolledBack(_) => {
                    std::process::exit(1)
                }
            }
        }
        Err(err) => {
            eprintln!("backfill failed: {err}");
            std::process::exit(1);
        }
    }
}
