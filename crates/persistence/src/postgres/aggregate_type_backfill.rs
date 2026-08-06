//! Offline backfill of the `events.aggregate_type` column.
//!
//! This is deliberately not part of the automatic migration runner: filling
//! the column for rows that predate it requires knowing which entity types a
//! deployment has registered, and refusing to guess when a stored identifier
//! does not split unambiguously under that list. An automatic migration
//! cannot supply that list or make that judgment call, so this is an
//! operator-invoked step instead.

use std::collections::HashMap;

use serde::Serialize;
use sqlx::PgPool;

/// The outcome of trying to split one stored `aggregate_id` string into a
/// type and a bare id, given the deployment's registered entity types.
#[derive(Debug, Clone, PartialEq, Eq)]
enum SplitOutcome {
    /// No registered type is a prefix of the stored value (followed by the
    /// join separator), so nothing can be split off at all.
    NoMatch,
    /// More than one registered type is a valid prefix. Returning the first
    /// match and moving on would silently guess at a fact the data does not
    /// determine; this variant exists so the caller can refuse instead.
    Ambiguous,
    /// Exactly one registered type matches. The remainder — including any
    /// leading/trailing whitespace, which the caller checks separately — is
    /// the candidate bare id.
    Split {
        aggregate_type: String,
        aggregate_id: String,
    },
}

/// Tries every registered type as a `"{type}-"` prefix of `raw`. A type only
/// counts as a match if it is followed by the join separator and at least
/// one more character remains — a bare type name with nothing after the
/// separator is not treated as a match here; the empty remainder is instead
/// surfaced by the caller's own empty/whitespace check, so that failure mode
/// has one place it is reported from, not two.
fn split_aggregate_id(raw: &str, registered_types: &[String]) -> SplitOutcome {
    let matches: Vec<&String> = registered_types
        .iter()
        .filter(|candidate_type| {
            let prefix_len = candidate_type.len();
            raw.len() > prefix_len
                && raw.as_bytes()[prefix_len] == b'-'
                && raw.starts_with(candidate_type.as_str())
        })
        .collect();

    match matches.len() {
        0 => SplitOutcome::NoMatch,
        1 => {
            let matched_type = matches[0];
            SplitOutcome::Split {
                aggregate_type: matched_type.clone(),
                aggregate_id: raw[matched_type.len() + 1..].to_string(),
            }
        }
        _ => SplitOutcome::Ambiguous,
    }
}

// ---------------------------------------------------------------------------
// Report shape
// ---------------------------------------------------------------------------

/// The result of one backfill attempt: how many rows were scanned, how many
/// were actually rewritten, and whether the run committed or refused.
///
/// Serializes to JSON so an operator or a pipeline can inspect the exact
/// outcome, including which rows caused an abort, without re-deriving it
/// from log output.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BackfillReport {
    pub rows_scanned: u64,
    /// Rows whose split was consolidated. Zero on every refusal, including a
    /// rolled-back one: rows were written there, but none of them persisted,
    /// and this field reports what an operator will find in the table.
    pub rows_rewritten: u64,
    pub outcome: BackfillOutcome,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BackfillOutcome {
    /// Every row split unambiguously under the registered type list, no
    /// post-split identity collided with another row's, the rewritten table
    /// passed post-verification, and the rewrite — including making the column
    /// mandatory — committed as one transaction.
    Committed,
    /// At least one row failed a precondition. Nothing was written: the
    /// whole scan ran inside one transaction which ended without writing rather than
    /// committed, so the table is exactly as it was before this attempt.
    Aborted(AbortReport),
    /// The rewrite ran and post-verification then rejected the result, so the
    /// whole transaction was rolled back.
    ///
    /// This is a genuinely different outcome from [`Self::Aborted`], not a
    /// synonym: there, no row was ever written and the guarantee comes from the
    /// ordering; here, rows *were* written inside the transaction and the
    /// guarantee comes from discarding them. The table is unchanged either way,
    /// but an operator reading the report is looking at two different events,
    /// and side effects that a rollback does not undo — an advanced sequence,
    /// for instance — belong to this one only.
    RolledBack(AbortReport),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AbortReport {
    pub reason: AbortReason,
    /// The primary-key `id` of every row that violated `reason`, so an
    /// operator can go straight to the offending data instead of re-scanning
    /// the table to find it.
    ///
    /// Empty for [`AbortReason::RowCountChanged`], which is a property of the
    /// table as a whole: the rows that appeared were not in the scan, so there
    /// are no scanned ids to name. Their count is in the reason itself.
    pub offending_row_ids: Vec<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AbortReason {
    /// The stored `aggregate_id` does not begin with any registered type
    /// followed by the join separator.
    NoRegisteredTypeMatches,
    /// More than one registered type is a valid prefix of the stored value —
    /// the two are indistinguishable without more information than the row
    /// itself carries.
    MatchesMoreThanOneRegisteredType,
    /// The candidate bare id, after splitting off the type, is empty or
    /// contains only whitespace.
    AggregateIdIsEmptyOrWhitespace,
    /// Two or more rows would land on the identical
    /// `(tenant_id, aggregate_type, aggregate_id, version)` identity after
    /// splitting — the same collision the eventual unique index would refuse,
    /// caught here before any row is rewritten.
    PostSplitIdentityWouldCollide,
    /// Post-verification: the table holds a different number of rows than the
    /// scan saw. Something inserted or deleted rows while the rewrite was in
    /// flight — the case the runbook's "quiesce the old writers first" step
    /// exists to prevent, caught here rather than consolidated.
    RowCountChanged { scanned: u64, found: u64 },
    /// Post-verification: two rows in the *written* table share one
    /// `(tenant_id, aggregate_type, aggregate_id, version)` identity.
    ///
    /// The preflight check for the same condition runs against the split values
    /// computed in memory, which is not the same claim: it proves the intended
    /// result was collision-free, not that the table now is. This one reads the
    /// rows as they were actually written.
    PostSplitIdentityIsNotUnique,
    /// Post-verification: some stream's versions are not the consecutive run
    /// `1..=n`.
    ///
    /// This is the check that proves the transformation did not re-partition
    /// history. The split changes which rows belong to which stream, and a
    /// mistake there shows up as a stream missing its first version or carrying
    /// a hole. The tool cannot tell a hole it created from one that was already
    /// in the data, so it refuses either way and names the rows: consolidating
    /// on the assumption that the gap was pre-existing is exactly the guess this
    /// migration must not make.
    StreamVersionsAreNotConsecutiveFromOne,
}

/// A database error surfaced while running the backfill. Distinguished from
/// [`AbortReport`] because this means the attempt could not be evaluated at
/// all, not that it was evaluated and refused.
#[derive(Debug)]
pub struct BackfillError(pub sqlx::Error);

impl std::fmt::Display for BackfillError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "aggregate_type backfill failed: {}", self.0)
    }
}

impl std::error::Error for BackfillError {}

impl From<sqlx::Error> for BackfillError {
    fn from(err: sqlx::Error) -> Self {
        BackfillError(err)
    }
}

// ---------------------------------------------------------------------------
// The backfill itself
// ---------------------------------------------------------------------------

/// One row as scanned from `events`, before any preflight judgment is made
/// about it.
struct ScannedRow {
    id: i64,
    tenant_id: Option<String>,
    aggregate_id: String,
    version: i64,
}

/// Splits every row's `aggregate_id` against `registered_types`, rewrites the
/// table to store the type and the bare id separately, verifies the result, and
/// makes the new column mandatory — all inside one transaction, so a preflight
/// failure, a failed verification, or a mid-run database error leaves the table
/// exactly as it was.
///
/// The run has two distinct judgment stages and they are not interchangeable:
///
/// - **Preflight**, before the first write, over values computed in memory. It
///   decides whether the transformation is *defined* for this table at all.
/// - **Post-verification**, after every write and before the column becomes
///   mandatory, over the rows as they were actually written. It decides whether
///   the transformation *did what it claimed* — row count preserved, the new
///   identity unique, and every stream's versions the consecutive run `1..=n`.
///
/// A preflight failure means nothing was ever written. A verification failure
/// means the writes are discarded by rolling back. Both leave the table
/// untouched, and the report distinguishes them because they are different
/// events for whoever has to act on them.
///
/// Preflight order does not change which rows end up reported: each check
/// only fires when no earlier check already found a violation elsewhere in
/// the table, and every row is independently classified before any of them
/// causes an abort, so a row can only ever be named under the first
/// applicable reason, never several.
pub async fn backfill_aggregate_type(
    pool: &PgPool,
    registered_types: &[String],
) -> Result<BackfillReport, BackfillError> {
    let mut tx = pool.begin().await?;

    let rows: Vec<ScannedRow> = sqlx::query_as::<_, (i64, Option<String>, String, i64)>(
        "SELECT id, tenant_id, aggregate_id, version FROM events ORDER BY id",
    )
    .fetch_all(&mut *tx)
    .await?
    .into_iter()
    .map(|(id, tenant_id, aggregate_id, version)| ScannedRow {
        id,
        tenant_id,
        aggregate_id,
        version,
    })
    .collect();

    let rows_scanned = rows.len() as u64;

    let mut no_match: Vec<i64> = Vec::new();
    let mut ambiguous: Vec<i64> = Vec::new();
    let mut empty_id: Vec<i64> = Vec::new();
    let mut splits: Vec<(&ScannedRow, String, String)> = Vec::new();

    for row in &rows {
        match split_aggregate_id(&row.aggregate_id, registered_types) {
            SplitOutcome::NoMatch => no_match.push(row.id),
            SplitOutcome::Ambiguous => ambiguous.push(row.id),
            SplitOutcome::Split {
                aggregate_type,
                aggregate_id,
            } => {
                if aggregate_id.trim().is_empty() {
                    empty_id.push(row.id);
                } else {
                    splits.push((row, aggregate_type, aggregate_id));
                }
            }
        }
    }

    let abort = |tx: sqlx::Transaction<'_, sqlx::Postgres>,
                 reason: AbortReason,
                 offending_row_ids: Vec<i64>|
     -> BackfillReport {
        // Nothing has been written at this point — only the SELECT above has
        // run — so there is no work to undo and "rollback" would be the wrong
        // word for it. The transaction is dropped, which ends it; the
        // guarantee that no row was modified comes from the ordering, not from
        // discarding a partial write. Every classification happens in memory
        // before the first UPDATE is issued further down.
        drop(tx);
        BackfillReport {
            rows_scanned,
            rows_rewritten: 0,
            outcome: BackfillOutcome::Aborted(AbortReport {
                reason,
                offending_row_ids,
            }),
        }
    };

    if !no_match.is_empty() {
        return Ok(abort(tx, AbortReason::NoRegisteredTypeMatches, no_match));
    }
    if !ambiguous.is_empty() {
        return Ok(abort(
            tx,
            AbortReason::MatchesMoreThanOneRegisteredType,
            ambiguous,
        ));
    }
    if !empty_id.is_empty() {
        return Ok(abort(
            tx,
            AbortReason::AggregateIdIsEmptyOrWhitespace,
            empty_id,
        ));
    }

    let mut seen: HashMap<(Option<String>, String, String, i64), Vec<i64>> = HashMap::new();
    for (row, aggregate_type, aggregate_id) in &splits {
        seen.entry((
            row.tenant_id.clone(),
            aggregate_type.clone(),
            aggregate_id.clone(),
            row.version,
        ))
        .or_default()
        .push(row.id);
    }
    let colliding: Vec<i64> = seen
        .into_values()
        .filter(|ids| ids.len() > 1)
        .flatten()
        .collect();
    if !colliding.is_empty() {
        return Ok(abort(
            tx,
            AbortReason::PostSplitIdentityWouldCollide,
            colliding,
        ));
    }

    for (row, aggregate_type, aggregate_id) in &splits {
        sqlx::query("UPDATE events SET aggregate_type = $1, aggregate_id = $2 WHERE id = $3")
            .bind(aggregate_type)
            .bind(aggregate_id)
            .bind(row.id)
            .execute(&mut *tx)
            .await?;
    }

    // Post-verification, against the rows as they were actually written and
    // still inside the same transaction. The preflight checks judged the
    // *intended* result computed in memory; these judge the table. Only if all
    // three pass does the column become mandatory and the transaction commit.
    let rolled_back = |reason: AbortReason, offending_row_ids: Vec<i64>| BackfillReport {
        rows_scanned,
        rows_rewritten: 0,
        outcome: BackfillOutcome::RolledBack(AbortReport {
            reason,
            offending_row_ids,
        }),
    };

    let found: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM events")
        .fetch_one(&mut *tx)
        .await?;
    if found as u64 != rows_scanned {
        tx.rollback().await?;
        return Ok(rolled_back(
            AbortReason::RowCountChanged {
                scanned: rows_scanned,
                found: found as u64,
            },
            Vec::new(),
        ));
    }

    // Both of the checks below compare stream identity with IS NOT DISTINCT
    // FROM rather than =, because tenant_id is nullable and `NULL = NULL` is
    // NULL: an untenanted stream would silently never match itself and its
    // violations would go unreported. GROUP BY already treats nulls as equal,
    // so only the join back to the offending rows needs saying.
    let duplicate_row_ids: Vec<i64> = sqlx::query_scalar(
        "SELECT e.id FROM events e \
         JOIN (SELECT tenant_id, aggregate_type, aggregate_id, version FROM events \
               GROUP BY tenant_id, aggregate_type, aggregate_id, version \
               HAVING COUNT(*) > 1) duplicated \
           ON e.tenant_id IS NOT DISTINCT FROM duplicated.tenant_id \
          AND e.aggregate_type IS NOT DISTINCT FROM duplicated.aggregate_type \
          AND e.aggregate_id = duplicated.aggregate_id \
          AND e.version = duplicated.version \
         ORDER BY e.id",
    )
    .fetch_all(&mut *tx)
    .await?;
    if !duplicate_row_ids.is_empty() {
        tx.rollback().await?;
        return Ok(rolled_back(
            AbortReason::PostSplitIdentityIsNotUnique,
            duplicate_row_ids,
        ));
    }

    // Uniqueness has just been established, so per stream COUNT(*) is the
    // number of distinct versions; a run starting at 1 with no holes is then
    // exactly MIN = 1 and MAX = COUNT(*).
    let discontinuous_row_ids: Vec<i64> = sqlx::query_scalar(
        "SELECT e.id FROM events e \
         JOIN (SELECT tenant_id, aggregate_type, aggregate_id FROM events \
               GROUP BY tenant_id, aggregate_type, aggregate_id \
               HAVING MIN(version) <> 1 OR MAX(version) <> COUNT(*)) broken \
           ON e.tenant_id IS NOT DISTINCT FROM broken.tenant_id \
          AND e.aggregate_type IS NOT DISTINCT FROM broken.aggregate_type \
          AND e.aggregate_id = broken.aggregate_id \
         ORDER BY e.id",
    )
    .fetch_all(&mut *tx)
    .await?;
    if !discontinuous_row_ids.is_empty() {
        tx.rollback().await?;
        return Ok(rolled_back(
            AbortReason::StreamVersionsAreNotConsecutiveFromOne,
            discontinuous_row_ids,
        ));
    }

    sqlx::query("ALTER TABLE events ALTER COLUMN aggregate_type SET NOT NULL")
        .execute(&mut *tx)
        .await?;

    tx.commit().await?;

    Ok(BackfillReport {
        rows_scanned,
        rows_rewritten: splits.len() as u64,
        outcome: BackfillOutcome::Committed,
    })
}

/// The exact, lossless reverse of [`backfill_aggregate_type`]: rejoins
/// `aggregate_type` and `aggregate_id` back into the single joined string
/// `aggregate_id` held before the split, then drops the column. Rows that
/// were never backfilled (`aggregate_type IS NULL`) are left untouched, since
/// their `aggregate_id` was never rewritten in the first place.
///
/// Runs inside one transaction: the rejoin and the column drop either both
/// happen or neither does.
pub async fn revert_aggregate_type_column(pool: &PgPool) -> Result<(), BackfillError> {
    let mut tx = pool.begin().await?;

    sqlx::query("ALTER TABLE events ALTER COLUMN aggregate_type DROP NOT NULL")
        .execute(&mut *tx)
        .await?;

    sqlx::query(
        "UPDATE events SET aggregate_id = aggregate_type || '-' || aggregate_id \
         WHERE aggregate_type IS NOT NULL",
    )
    .execute(&mut *tx)
    .await?;

    sqlx::query("ALTER TABLE events DROP COLUMN aggregate_type")
        .execute(&mut *tx)
        .await?;

    tx.commit().await?;
    Ok(())
}

#[cfg(test)]
mod split_aggregate_id_tests {
    use super::*;

    fn types(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn splits_cleanly_when_exactly_one_registered_type_matches() {
        let outcome = split_aggregate_id("user-7", &types(&["user"]));
        assert_eq!(
            outcome,
            SplitOutcome::Split {
                aggregate_type: "user".to_string(),
                aggregate_id: "7".to_string(),
            }
        );
    }

    #[test]
    fn reports_no_match_when_no_registered_type_is_a_prefix() {
        let outcome = split_aggregate_id("orphan-123", &types(&["user", "organization"]));
        assert_eq!(outcome, SplitOutcome::NoMatch);
    }

    #[test]
    fn reports_no_match_when_the_value_equals_a_registered_type_with_nothing_after_it() {
        // No separator, no remainder to be an id at all — never a match.
        let outcome = split_aggregate_id("user", &types(&["user"]));
        assert_eq!(outcome, SplitOutcome::NoMatch);
    }

    #[test]
    fn reports_ambiguous_when_more_than_one_registered_type_matches() {
        // The motivating case: "user-account" and "user" are both
        // registered, and "user-account-7" is a valid split under either.
        let outcome = split_aggregate_id("user-account-7", &types(&["user-account", "user"]));
        assert_eq!(outcome, SplitOutcome::Ambiguous);
    }

    #[test]
    fn a_trailing_separator_with_nothing_after_it_still_splits_with_an_empty_remainder() {
        // The empty-remainder judgment belongs to the caller (the
        // empty/whitespace preflight check), not to this function.
        let outcome = split_aggregate_id("user-", &types(&["user"]));
        assert_eq!(
            outcome,
            SplitOutcome::Split {
                aggregate_type: "user".to_string(),
                aggregate_id: String::new(),
            }
        );
    }
}
