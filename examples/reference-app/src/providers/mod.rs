//! CORE-019A Phase 6 dogfood providers — outbound `ExternalDataProvider`
//! implementations, kept out of `domain` so the domain layer never defines
//! the concrete provider a handler reaches only through the
//! `DataProviderAccess` facade (`external_data_provider_lint.rs` audits
//! `domain/` on exactly that separation).

pub mod pricing_lookup;
