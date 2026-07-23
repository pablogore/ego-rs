//! Per-crate isolation compilation (design.md AD-4, FR-005).

/// Runs `cargo check -p <crate> --no-default-features` for every crate in
/// `crates`, returning the names of any that fail. No crate in this
/// workspace declares a `default` feature, so `--no-default-features` is a
/// harmless strict floor rather than a behavior change.
pub fn verify_isolation(crates: &[String]) -> anyhow::Result<Vec<String>> {
    if crates.is_empty() {
        anyhow::bail!("verify-isolation: crate list is empty, refusing a vacuous pass");
    }
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string());
    let mut failures = Vec::new();
    for name in crates {
        let status = std::process::Command::new(&cargo)
            .args(["check", "-p", name, "--no-default-features"])
            .status()
            .map_err(|e| anyhow::anyhow!("running cargo check -p {name}: {e}"))?;
        if !status.success() {
            failures.push(name.clone());
        }
    }
    Ok(failures)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_crate_list_is_an_error_not_a_vacuous_pass() {
        let result = verify_isolation(&[]);
        assert!(
            result.is_err(),
            "empty crate list must fail loudly, not silently pass"
        );
    }
}
