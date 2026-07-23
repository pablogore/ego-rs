mod cycles;
mod hygiene;
mod isolation;
mod layers;
mod metadata;

fn main() -> anyhow::Result<()> {
    let cmd = std::env::args().nth(1);
    let passed = match cmd.as_deref() {
        Some("verify-layers") => verify_layers()?,
        Some("verify-isolation") => verify_isolation()?,
        Some("verify-hygiene") => verify_hygiene()?,
        _ => {
            eprintln!(
                "usage: cargo run -p xtask -- <verify-layers|verify-isolation|verify-hygiene>"
            );
            std::process::exit(2);
        }
    };
    std::process::exit(if passed { 0 } else { 1 });
}

/// FR-001/FR-002/FR-003/FR-004: direction + cycles + completeness, one
/// human-readable report, `Ok(true)` clean / `Ok(false)` any violation.
fn verify_layers() -> anyhow::Result<bool> {
    let workspace = metadata::load_workspace()?;
    let layers_toml = metadata::workspace_root().join("layers.toml");
    let layer_map = layers::load_layers_toml(&layers_toml)?;

    let mut violations = layers::check_completeness(&workspace.crates, &layer_map);
    violations.extend(layers::check_direction(&workspace.graph, &layer_map));
    violations.extend(
        cycles::find_cycles(&workspace.graph)
            .into_iter()
            .map(layers::Violation::Cycle),
    );

    if violations.is_empty() {
        println!(
            "verify-layers: OK ({} crates, 0 violations)",
            workspace.crates.len()
        );
        return Ok(true);
    }

    println!("verify-layers: FAIL ({} violation(s))", violations.len());
    for v in &violations {
        println!("  - {v}");
    }
    Ok(false)
}

/// FR-005: every `crates/*` member compiles under its own narrowest feature
/// set, independent of workspace feature unification.
fn verify_isolation() -> anyhow::Result<bool> {
    let workspace = metadata::load_workspace()?;
    let failures = isolation::verify_isolation(&workspace.crates)?;

    if failures.is_empty() {
        println!(
            "verify-isolation: OK ({} crates checked in isolation)",
            workspace.crates.len()
        );
        return Ok(true);
    }

    println!(
        "verify-isolation: FAIL ({} crate(s) failed in isolation)",
        failures.len()
    );
    for name in &failures {
        println!("  - {name}");
    }
    Ok(false)
}

/// FR-006: no un-archived duplicate of an already-archived change.
fn verify_hygiene() -> anyhow::Result<bool> {
    let changes_dir = metadata::workspace_root().join("openspec/changes");
    let duplicates = hygiene::check_hygiene(&changes_dir)?;

    if duplicates.is_empty() {
        println!("verify-hygiene: OK (no un-archived duplicates)");
        return Ok(true);
    }

    println!("verify-hygiene: FAIL ({} duplicate(s))", duplicates.len());
    for name in &duplicates {
        println!("  - {name} duplicates an already-archived change");
    }
    Ok(false)
}
