//! Stale-change hygiene check (design.md AD-5, FR-006): an un-archived
//! `openspec/changes/<name>` dir MUST NOT case-insensitively suffix-match an
//! `archive/<YYYY-MM-DD>-<name>` dir.

use std::path::Path;

/// Returns the names of un-archived dirs under `changes_dir` that duplicate
/// an already-archived change.
pub fn check_hygiene(changes_dir: &Path) -> anyhow::Result<Vec<String>> {
    let archive_dir = changes_dir.join("archive");
    let mut archived_suffixes: Vec<String> = Vec::new();
    if archive_dir.is_dir() {
        for entry in std::fs::read_dir(&archive_dir)
            .map_err(|e| anyhow::anyhow!("reading {}: {e}", archive_dir.display()))?
        {
            let entry = entry?;
            if !entry.file_type()?.is_dir() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().to_string();
            archived_suffixes.push(strip_date_prefix(&name).to_lowercase());
        }
    }

    let mut duplicates = Vec::new();
    for entry in std::fs::read_dir(changes_dir)
        .map_err(|e| anyhow::anyhow!("reading {}: {e}", changes_dir.display()))?
    {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        if name == "archive" {
            continue;
        }
        if archived_suffixes.contains(&name.to_lowercase()) {
            duplicates.push(name);
        }
    }
    duplicates.sort();
    Ok(duplicates)
}

/// Strips a leading `YYYY-MM-DD-` date prefix, if present.
fn strip_date_prefix(name: &str) -> &str {
    let bytes = name.as_bytes();
    let has_prefix = bytes.len() > 11
        && bytes[0..4].iter().all(u8::is_ascii_digit)
        && bytes[4] == b'-'
        && bytes[5..7].iter().all(u8::is_ascii_digit)
        && bytes[7] == b'-'
        && bytes[8..10].iter().all(u8::is_ascii_digit)
        && bytes[10] == b'-';
    if has_prefix {
        &name[11..]
    } else {
        name
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn fails_when_unarchived_dir_suffix_matches_an_archived_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let changes_dir = tmp.path();
        fs::create_dir_all(changes_dir.join("archive/2026-07-15-core-019-reliable-external-effects"))
            .unwrap();
        fs::create_dir_all(changes_dir.join("core-019-reliable-external-effects")).unwrap();

        let duplicates = check_hygiene(changes_dir).unwrap();

        assert_eq!(duplicates, vec!["core-019-reliable-external-effects".to_string()]);
    }

    #[test]
    fn passes_when_no_unarchived_duplicate_exists() {
        let tmp = tempfile::tempdir().unwrap();
        let changes_dir = tmp.path();
        fs::create_dir_all(changes_dir.join("archive/2026-07-15-core-019-reliable-external-effects"))
            .unwrap();
        fs::create_dir_all(changes_dir.join("core-020-something-else")).unwrap();

        let duplicates = check_hygiene(changes_dir).unwrap();

        assert!(duplicates.is_empty());
    }
}
