//! Event adapter — converts between protobuf events, CloudEvents, and EventStore format.
//!
//! Provides the conversion pipeline:
//! `protobuf → CloudEvent → EventStore → EventStreamElement`
