---
ticket_id: TASK-026
spec_id: SPEC-002
module_id: workspace-git
constitution_version: 2.0
status: complete
parallel_safe: false
depends_on:
  - TASK-012
  - TASK-014
allowed_paths:
  - Cargo.toml
  - Cargo.lock
  - crates/lattice-workspace-git/**
  - docs/modules/workspace-git/MODULE_CONSTITUTION.md
  - docs/tickets/TASK-026-workspace-git-2.md
likely_files:
  - crates/lattice-workspace-git/Cargo.toml
  - crates/lattice-workspace-git/src/lib.rs
  - crates/lattice-workspace-git/tests/disposable_git.rs
branch: feature/task-026-027-workspace-scope
---

## Objective

Deliver Workspace Git 2.0 as one Rust Git/worktree evidence boundary that
consumes formal Project Registry identity and Writer Lease/fencing capability,
uses no second lease or Git truth, and is exercised only against disposable
repositories.

## Acceptance Criteria

- [x] Contributes the Workspace Git portion of SPEC-002 v27 AC-06: exact
  Registry/Lease binding, exact-base worktree creation, normalized changed-path
  evidence, path-kind evidence, conflict fail-closed, and exact clean cleanup.
- [x] Every Git process uses executable plus argument arrays with hooks,
  fsmonitor, inherited config injection, includes, and external drivers denied.
- [x] No code path invokes shell interpolation, automatic conflict resolution,
  reset, clean, stash, forced removal, or broad cleanup.
- [x] A local guard is visibly non-authoritative and every operation rechecks
  the formal authority pair and exact Git/worktree identity.
- [x] Existing Node workspace tests remain passing characterization evidence.

## Non-Goals

Durable Registry/Lease persistence, issuing or refreshing a lease/token, Policy
authorization, Scope Check classification, task orchestration, user-repository
testing, conflict repair, merge/ref update, push, primary merge, deployment, or
release.

## Module And Constitution Constraints

Use Workspace Git 2.0. Production code may consume Project Registry's immutable
public `RepositoryObservation`, shared authority contracts, and canonical hash
mechanics, but it may not access mutable Registry state or depend on the Writer
Lease owner implementation. Tests must obtain authority from the formal fake
owners and must label that evidence `RuntimeKind::Fake`.

## Dependencies And Overlap

TASK-012 and TASK-014 are complete and expose the formal owner receipts/current
heads required here. TASK-022 durable Registry and future durable Writer Lease
remain separate persistence work and are not reimplemented. TASK-038 MCP work
is not a dependency. TASK-027 depends on this ticket's frozen normalized-change
contract. This ticket owns only `crates/lattice-workspace-git/**` and its own
constitution/ticket; root Cargo files are sequential registration surfaces and
are not edited concurrently with TASK-027.

## TDD Behaviors

1. Reject substituted/stale/non-active Registry or Lease evidence and every
   project/snapshot/worktree/fencing mismatch before a Git effect.
2. Create one exact-base task worktree from the registered root and prove all
   Git invocations are argument arrays with protected configuration.
3. Produce stable committed/staged/unstaged/untracked/ignored/delete/rename/
   type-change/unmerged path and link/reparse evidence.
4. Return clean/conflict merge-readiness evidence without changing checkouts or
   refs and without resolving conflicts.
5. Remove only the exact clean process-owned worktree under matching current
   authority; stale, dirty, linked, or ambiguous state fails closed.

## Verification

| Check | Command or service | Expected evidence |
|---|---|---|
| Focused unit/integration | `cargo test -p lattice-workspace-git --all-targets --locked` | exit 0; disposable fixtures only |
| Strict lint | `cargo clippy -p lattice-workspace-git --all-targets --all-features --locked -- -D warnings` | exit 0 |
| Format | `cargo fmt --all -- --check` | exit 0 |
| Node characterization | `npm.cmd run verify` | exit 0 |

## Human Gate

none; the V2 amendment and bounded local implementation were already approved.
