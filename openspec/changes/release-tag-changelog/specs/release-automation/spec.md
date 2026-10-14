# Release Automation Specification

> Canonical / English. Spanish companion: `spec.es.md` (1:1 requirement IDs and scenarios).

## Purpose

Defines what constitutes a release of this repository: when exactly one is cut, how its version is
derived from commit history, and what the published tag and GitHub Release must observably
contain. Scoped to promotions onto `main` only; nothing here governs `develop`, feature branches,
or crate-level versioning.

## Requirements

### Requirement: A Release Is Cut Only, And Exactly Once, As A Direct Result Of A Push Landing On Main

The system MUST create exactly one new git tag and exactly one new GitHub Release as a direct
result of a change landing on `main`. The system MUST NOT create a tag or a Release as a result of
a change landing on `develop`, on any other branch, or on an open or updated pull request.

#### Scenario: A merge to main produces exactly one tag and one release

- GIVEN a pull request is merged into `main`
- WHEN the merge lands
- THEN exactly one new git tag exists pointing at the resulting commit, and exactly one new
  GitHub Release exists referencing that tag

#### Scenario: A merge to develop produces no tag or release

- GIVEN a pull request is merged into `develop`
- WHEN the merge lands
- THEN no new git tag and no new GitHub Release exist as a result

#### Scenario: An open or updated pull request produces no tag or release

- GIVEN a pull request is opened or updated against any branch
- WHEN CI runs for that pull request
- THEN no new git tag and no new GitHub Release is created

### Requirement: Version Is Derived From Conventional Commits Since The Previous Release, Staying In 0.x

The version applied to a release tag MUST be computed from the Conventional Commits reachable in
the range between the previous release tag (exclusive) and the new release's commit (inclusive).
The computed version MUST follow semantic versioning for that range's commit types and MUST remain
within the `0.x` major line unless and until an explicit decision changes that baseline. Before any
prior release tag exists, the repository's version MUST start at `v0.1.0`.

#### Scenario: The first release seeds the baseline version

- GIVEN no prior release tag exists on `main`
- WHEN the first release is cut
- THEN the resulting tag is `v0.1.0`

#### Scenario: A commit range determines the next version

- GIVEN a range of Conventional Commits since the previous release tag
- WHEN a release is cut
- THEN the new tag's version reflects semantic versioning applied to that range's commit types,
  and remains within the `0.x` major line

### Requirement: Release Notes Publish Only To The GitHub Release Body, Never To The Repository Tree

The generated changelog content for a release MUST be published as the body of that release's
GitHub Release. No commit adding, modifying, or removing any file MUST be created in the
repository as part of cutting a release, and no commit MUST be pushed to `main` or `develop` as a
result of the release process. No branch protection rule on any branch MUST be changed, relaxed,
or bypassed as a result of the release process.

#### Scenario: The release body carries the changelog content

- GIVEN a release is cut for a range of Conventional Commits
- WHEN the GitHub Release is created
- THEN its body contains the changelog content for that range

#### Scenario: No file changes and no commit result from cutting a release

- GIVEN a release has just been cut
- WHEN the repository tree and branch history are inspected
- THEN no file was added, modified, or removed, and no commit was pushed to `main` or `develop`
  as part of the release process

### Requirement: The Release Body Is A Human-Readable Record Grouped By Conventional-Commit Type

The release body MUST list the Conventional Commits in the release's range, grouped by their
commit type (for example: features, fixes, breaking changes), in a form readable by a person
without consulting git directly.

#### Scenario: The release body groups commits by type

- GIVEN a release whose range contains commits of more than one Conventional-Commit type
- WHEN the release body is read
- THEN the commits appear grouped by type, and a reader can identify what changed without
  running any git command

### Requirement: Cutting A Release Creates No Push-Triggered Retrigger Loop On Main

Because cutting a release performs no push to any branch, it MUST NOT itself cause another run of
any process that triggers on a push to `main`.

#### Scenario: No secondary push-triggered run is caused by a release

- GIVEN a release has just been cut as a result of a push to `main`
- WHEN the release process completes
- THEN no additional push event to `main` is generated, and no additional run of a
  push-to-main-triggered process is caused by the release itself

### Requirement: Release Versioning Is Independent Of Crate Manifests

The version applied to a release tag MUST be independent of, and MUST NOT modify, any crate
manifest version in the repository.

#### Scenario: No manifest changes result from cutting a release

- GIVEN a release has just been cut
- WHEN every crate manifest in the workspace is inspected
- THEN none of their version fields changed as a result

## Non-Goals

- Per-crate versioning or publishing to crates.io.
- A tracked `CHANGELOG.md` file in the repository tree.
- Changes to `production-gate.yml`'s gating behavior or triggers.
- Signed commits, linear-history enforcement, or any change to branch protection configuration.
- A release cut from any branch other than `main`, or more than one release per promotion.
