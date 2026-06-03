# Contract: Migration Infrastructure

## Shared Infrastructure (ego-infrastructure)

```rust
pub struct Migration {
    pub version: i64,
    pub description: &'static str,
    pub up: fn(&mut dyn MigrationContext) -> Result<(), Box<dyn std::error::Error>>,
    pub down: Option<fn(&mut dyn MigrationContext) -> Result<(), Box<dyn std::error::Error>>>,
}

pub trait MigrationContext {
    fn execute(&mut self, sql: &str) -> Result<(), Box<dyn std::error::Error>>;
    fn applied_versions(&self) -> Result<Vec<i64>, Box<dyn std::error::Error>>;
}

pub struct MigrationRegistry {
    migrations: Vec<Migration>,
}

impl MigrationRegistry {
    pub fn register(&mut self, migration: Migration) -> &mut Self;
    pub fn validate(&self) -> Result<(), MigrationError>;
    pub fn run(&self, ctx: &mut dyn MigrationContext) -> Result<(), MigrationError>;
}
```

## Behavioral Contract

- Migrations execute in monotonically increasing version order
- Already-applied migrations are idempotent (no-op on re-run)
- System refuses to start if pending migrations exist
- Invalid schema prevents startup (fail closed)
