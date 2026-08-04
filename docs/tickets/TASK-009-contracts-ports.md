---
ticket_id: TASK-009
spec_id: SPEC-002
spec_version: 3
module_id: lattice-contracts
constitution_version: 1.0
additional_modules:
  - module_id: lattice-ports
    constitution_version: 1.0
status: completed
parallel_safe: false
depends_on:
  - TASK-008
allowed_paths:
  - Cargo.toml
  - Cargo.lock
  - crates/lattice-contracts/**
  - crates/lattice-ports/**
  - PLANS.md
  - HANDOFF.md
  - docs/specs/SPEC-002-autonomous-development-platform.md
  - docs/modules/V2_AMENDMENT_PROPOSAL.md
  - docs/modules/README.md
  - docs/modules/lattice-contracts/**
  - docs/modules/lattice-ports/**
  - docs/tickets/TASK-009-contracts-ports.md
  - docs/workflow/WORKFLOW_LEDGER.md
  - docs/reviews/WORKFLOW_AUDIT_TASK_009_2026-07-29.md
  - docs/reviews/CODE_REVIEW_TASK_009_2026-07-29.md
  - docs/reviews/ARCHITECTURE_REVIEW_TASK_009_2026-07-29.md
  - docs/reviews/INTEGRATION_TASK_009_2026-07-29.md
likely_files:
  - Cargo.toml
  - crates/lattice-contracts/Cargo.toml
  - crates/lattice-contracts/src/lib.rs
  - crates/lattice-contracts/tests/contracts.rs
  - crates/lattice-ports/Cargo.toml
  - crates/lattice-ports/src/lib.rs
  - crates/lattice-ports/tests/ports.rs
branch: feature/v2-rust-postgres-bootstrap
---

## Objective

Create a dependency-safe Rust adapter boundary with immutable versioned
contracts, a typed inbound OpenClaw gateway service, and abstract outbound
ports for Codex, Graphify, Hermes, and the control store.

## Acceptance Criteria

- [x] Contract construction rejects empty IDs, malformed lowercase SHA-256
  references, and every unsupported contract version.
- [x] `lattice-ports` depends only on `lattice-contracts` and exposes abstract
  inbound gateway, code-writer, knowledge, research, and control-store traits;
  every trait returns a lane-specific evidence type.
- [x] OpenClaw is modeled as the inbound typed gateway service; it cannot send
  arbitrary shell, SQL, Git, provider, or product-path operations.
- [x] Codex is classified as product-code writer and each run request binds an
  exact writer-claim digest; Graphify is derived read-only evidence, Hermes is
  an untrusted candidate, and PostgreSQL is the future control-store
  implementation boundary.
- [x] Both crates use only workspace/local dependencies, perform no I/O, and
  remain `publish = false`.

## Non-Goals

- Implement task-domain transitions, canonical JSON, orchestration, policy,
  approval, artifact storage, fake providers, or real persistence.
- Connect to a database, external process, network, model, Git repository, or
  provider protocol.
- Activate the live OpenClaw, Codex, Graphify, Hermes, or PostgreSQL functional
  modules.

## Module And Constitution Constraints

- `lattice-contracts` 1.0 owns immutable boundary values and validation.
- `lattice-ports` 1.0 owns the five abstract traits and typed port errors.
- SPEC-002 version 3 and ADR-004/006 govern dependency direction and lane
  authority.

## Dependencies And Overlap

This ticket is not parallel-safe because it edits the root workspace manifest
and introduces shared contracts consumed by every later adapter ticket.

## TDD Behaviors

1. RED: contract tests fail before IDs, digest/version validation, invocation,
   and evidence types exist; GREEN: valid values round-trip and invalid inputs
   fail closed.
2. RED: port contract tests fail before five abstract traits and typed errors
   exist; GREEN: local test implementations compile against the contracts.
3. REVIEW RED: independent review proves generic `Evidence` permits
   component/boundary cross-labeling and port implementations still compile;
   GREEN: five lane-specific evidence return types make that mismatch a compile
   error and preserve normalized evidence conversion.

## Verification

| Check | Command or service | Expected evidence |
|---|---|---|
| Contracts | `cargo test -p lattice-contracts` | validation and identity tests pass |
| Ports | `cargo test -p lattice-ports` | five role contracts compile and pass |
| Format | `cargo fmt --check` | exit 0 |
| Lint | `cargo clippy --workspace --all-targets --all-features -- -D warnings` | exit 0 |
| Full Rust suite | `cargo test --workspace` | exit 0 |
| Dependency graph | `cargo metadata --format-version 1 --no-deps` | only approved local edges |
| Preserved Node suite | `npm.cmd run verify` | exit 0 |
| Scope and hygiene | allowed-path audit plus `git diff --check` | no foreign path; exit 0 |

## Human Gate

None for this local, reversible, dependency-only contract slice. Installation,
database execution, credentials, live provider calls, model use, payment,
commit/push/merge, publication, deployment, or public network exposure remain
separately gated.
