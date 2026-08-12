//! Shared fixtures for the infrastructure-backed suite.
//!
//! See `README.md` for the admission rules that govern what may live here. In
//! short: a scenario is admitted only if it traverses a capability end to end
//! and could fail in a way no in-process test can detect.
//!
//! This crate is a library so the fixtures — a shared PostgreSQL, migrations
//! applied once per run, per-test isolation — are defined once and reused,
//! rather than rebuilt per test file the way ad-hoc harnesses tend to be.
