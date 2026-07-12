//! Inbound adapters (hexagonal "ports") that drive the application core.
//! Only `http` exists today; a future adapter (e.g. gRPC) would sit
//! alongside it here, never inside the application or domain layers.

pub mod http;
