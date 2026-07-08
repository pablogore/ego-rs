# Proposal: CORE-022 — TestKit

## Motivation

ego.rs now has a full production stack: Runtime, Service SDK, Configuration,
Logger, Authorization, Security Context, API Keys, OAuth/OIDC. None of it
ships with an official way to exercise these contracts in tests. Every
project that consumes ego.rs is left to invent its own runtimes, contexts,
providers, and fixtures from scratch, using low-level building blocks that
were primarily designed for production composition.

## Problem Statement

Writing a test today means manually wiring a `Runtime`, a `ServiceContext`, an
`AuthorizationProvider`, a `SecurityContext`, an identity, a configuration
source, and a logger — by hand, per project, per test suite. This produces
three concrete problems:

- **Duplicated effort.** The same wiring is reinvented in every consuming
  project instead of solved once.
- **Divergence from production contracts.** Hand-rolled mocks drift from the
  real public contracts they stand in for, so a test can pass against a
  fake shape that production code no longer honors.
- **High entry cost.** Newcomers to a project must understand the internals
  of Runtime, Service SDK, and Security before they can write a single
  useful test, instead of expressing what they actually want to verify.

Project-local mocks do not solve this because they are not shared: each team
solves the same wiring problem independently, with no guarantee that two
projects' mocks agree with each other or with the real contracts. A fix or
improvement to one project's test doubles never reaches the others. Because
each project's test infrastructure is bespoke, a change to Runtime, Security,
or any other core contract risks breaking every project's mocks
independently, and each one has to be fixed separately. Examples,
documentation, and onboarding around testing are similarly stuck per project
instead of being shared across the ego.rs ecosystem.

## Goals

- Provide a consistent testing experience across the ego.rs ecosystem.
- Reduce the cost of writing and maintaining service tests.
- Ensure tests exercise the same public contracts as production.
- Eliminate duplicated testing infrastructure across consuming projects.

The concrete surface this capability covers — test configuration, a
capturable logger, identity builders, fixtures, assertion helpers, and
testing counterparts for Runtime, `ServiceContext`, `AuthorizationProvider`,
and `SecurityContext` — is a Spec-level concern, deliberately not enumerated
here.

## Non Goals

Out of scope for this capability, reserved for future work:

- HTTP testing
- gRPC testing
- Database testing
- Testcontainers
- Snapshot testing
- Property testing
- Benchmarks
- Performance testing
- Chaos testing

## Expected Benefits

- **Fewer bugs.** Tests built against the same contracts production uses
  catch real regressions instead of regressions in a mock that quietly
  stopped matching reality.
- **Lower cost per test suite.** Projects stop re-deriving the same
  runtime/context/provider wiring and start from a shared, maintained
  foundation.
- **Consistency across projects.** One canonical way to construct a test
  Runtime, Security Context, or Identity means test suites are legible
  across teams, not bespoke per project.
- **Better developer experience.** Contributors write tests that express
  intent — what is being verified — instead of boilerplate that assembles
  infrastructure.
- **A single point of improvement.** A fix or new fixture in TestKit benefits
  every consuming project at once, and a change to a core contract is
  accommodated once, in TestKit, instead of being rediscovered and patched in
  N bespoke mocks across N projects.
- **Confidence to refactor.** Engineers can change Runtime and Security
  internals with confidence because test suites depend on maintained public
  testing facilities, not on bespoke infrastructure that silently drifts.
- **A shared testing culture.** Examples, documentation, and onboarding
  around how to test an ego.rs service become reusable across projects
  instead of reinvented per team.
- **Part of the platform contract.** An official testing capability becomes
  part of the public platform contract, ensuring that testing evolves
  together with production APIs instead of independently within each
  consuming project.

## Scope

This proposal covers the case for an official TestKit capability: the problem it
solves, why per-project mocks are insufficient, and the benefits of a shared
capability built on ego.rs's existing public contracts (Runtime, Service SDK,
Configuration, Logger, Authorization, Security Context, API Keys,
OAuth/OIDC). It does not define APIs, structs, traits, pseudocode,
implementation, or internal modules — that is the concern of the Spec and
Design documents that follow, if this proposal is accepted. This proposal
intentionally focuses on the need for a shared testing capability rather than
the mechanics of how that capability is implemented.
