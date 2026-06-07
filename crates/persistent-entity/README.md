# Persistent Entity Runtime and SDK

This crate provides the core runtime and SDK for persistent entities in the EGO system.

## Features

- Entity lifecycle management
- Command processing with deterministic ordering
- State recovery and snapshotting
- Activation ordering guarantees
- Concurrency control and passivation policies

## Architecture

The persistent entity runtime implements a mailbox-based actor model with:
- Single-writer guarantee per entity
- Deterministic command processing
- Recovery barriers for state consistency
- Activation ordering invariants