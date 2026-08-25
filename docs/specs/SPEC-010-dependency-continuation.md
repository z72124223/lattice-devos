---
spec_id: SPEC-010
title: Durable dependency suspension and safe continuation
version: 1
status: approved
approved_by: explicit_user_delegation
approved_at_local: 2026-08-25
modules:
  - module_id: foreman-state
    constitution_version: 1.4
  - module_id: lattice-ports
    constitution_version: 2.2
  - module_id: latticed
    constitution_version: 3.2
  - module_id: postgres-store
    constitution_version: 1.20
  - module_id: workspace-git
    constitution_version: 1.1
---

# SPEC-010 — Durable dependency suspension and safe continuation

## Problem

The product can durably replay one foreman snapshot and can create bounded Git
worktrees, but it cannot durably identify a newly discovered dependency or
prove that the dependency was safely integrated before resuming the parent.
The current free-form blocker cannot restore the child identity, branch,
worktree locator, base SHA, or exact next action after a new process starts.

## Intended behavior

- Keep the canonical MCP surface at exactly seven tools. Extend only
  `lattice_foreman_checkpoint.blocker_ref` to accept either the existing
  string, `null`, or one closed `lattice.dependency-blocker/1.0` object.
- The object contains exactly `schema`, `parent_task_id`,
  `dependency_task_id`, `dependency_worktree_id`, `dependency_branch`,
  `base_sha`, and `next_action`. The branch must equal
  `lattice/<dependency-task-id-lowercase>`, `base_sha` is a lowercase 40-hex
  commit, and `next_action` is exactly `COMPLETE_DEPENDENCY`.
- Serialize that object into a canonical printable-ASCII blocker no longer
  than the existing 256-byte durable scalar. PostgreSQL and Task Ledger bytes
  remain unchanged. For object input, Runtime replaces the generic outer
  evidence pointer with a domain-separated canonical SHA-256 commitment to the
  binding. Replay promotes a complete canonical v1 scalar only when that
  PostgreSQL-retained commitment also matches; every non-canonical,
  version-colliding, or canonical-looking historical free string remains an
  opaque legacy blocker and stays replayable.
- A bounded CLI around the existing `GitWorkspace` creates the dependency
  worktree from the current clean product worktree at the exact requested base
  and returns only the closed binding fields. It accepts no command, hook,
  filter, merge driver, arbitrary branch, or target path.
- Before a new structured `BLOCKED` checkpoint is written, Runtime verifies
  the owned marker and live Git identity, exact branch/base, clean parent and
  child worktrees, and parent HEAD equal to the persisted base. Every probe
  pins an empty hooks directory, disables fsmonitor, and sets
  `GIT_OPTIONAL_LOCKS=0`.
- Before a later `ACTIVE` checkpoint can clear a structured blocker, Runtime
  replays the latest snapshots and verifies the same ownership, that parent and
  child HEAD descend from the stored base, and that child HEAD is an ancestor
  of parent HEAD. A dirty tree, mismatch, missing marker, duplicate/ambiguous
  worktree, conflict, changed base, or failed Git probe returns a stable error
  before Writer acquisition or Ledger append. The retained state stays
  `BLOCKED`. The exact parent branch/worktree/HEAD captured by that successful
  guard is the same server observation bound into the checkpoint; Runtime does
  not discard it and re-read a later HEAD.
- A structured dependency may leave `BLOCKED` only through a later verified
  `ACTIVE` checkpoint. A direct `BLOCKED` to `COMPLETED` transition is rejected
  before persistence; replay also fails closed if historical rows contain that
  contradictory sequence.
- Even when Runtime replaces a structured request's evidence with its own
  binding commitment, the caller-supplied outer `evidence_ref` remains required
  and must first satisfy the public lowercase SHA-256 pointer schema.
- Exact retry is resolved from verified PostgreSQL replay before any new Git
  observation and returns the original receipt even if live Git later changes.
- Zero-parameter Runtime Status adds a nullable `dependency` projection. It
  restores the parent and dependency task IDs, parent branch/worktree/base,
  an explicit `depends_on` equal to the dependency task ID, dependency
  branch/worktree locator/base, `BLOCKED|RESUMED`, and exact next action
  `COMPLETE_DEPENDENCY|CONTINUE_PARENT` from verified snapshot history.
  Live Git uncertainty may only change a separate verification field to
  `RECONCILIATION_REQUIRED`; it cannot silently report continuation-ready.

## Non-goals

- Generic DAG scheduling, multiple simultaneous writable children, recursive
  dependency spawning, automatic conflict resolution, automatic merge, or
  deletion of completed worktrees.
- A Task Domain transition out of terminal `Blocked`, a new database schema,
  caller-selected absolute paths, or a second durable registry.
- Restarting Codex App, force push, reset/clean, or modifying historical dirty
  worktrees.

## Failure and compatibility

Malformed or unknown structured fields fail as
`FOREMAN_CHECKPOINT_INVALID`. New-write Git mismatch fails as
`FOREMAN_DEPENDENCY_BINDING_MISMATCH`; missing/unsafe ownership fails as
`FOREMAN_DEPENDENCY_WORKTREE_UNSAFE`; dirty or conflicting state fails as
`FOREMAN_DEPENDENCY_RECONCILIATION_REQUIRED`; and unproven integration fails as
`FOREMAN_DEPENDENCY_NOT_INTEGRATED`. All are secret-free and append nothing.
Legacy string blockers keep the v1 Runtime projection behavior and produce a
`null` dependency projection.

## Acceptance criteria

- [ ] Pure tests cover canonical encode/decode, all bounds, unknown fields,
      branch derivation, malformed base, legacy compatibility, and replayed
      `BLOCKED` then `RESUMED` projection.
- [ ] Git tests create one owned child, reject dirty/root/path/marker/branch/base
      drift, prove a clean integration, and prove conflicts fail closed without
      modifying or deleting the child.
- [ ] The live cross-language gate uses the real Node dependency CLI to create
      the marker/worktree, then feeds that object through MCP and Rust Git guard;
      an exact retry returns its PostgreSQL receipt even after live Git drifts.
- [ ] MCP tests retain exact seven modern/two legacy tools and reject arbitrary
      paths, fields, branches, bases, and next actions before dispatch.
- [ ] PostgreSQL process A records the structured `BLOCKED` binding and stops;
      process B performs no bootstrap and returns the same binding/base/next
      action; after proven integration, process C records `ACTIVE`, and process
      D replays `RESUMED` plus `CONTINUE_PARENT`.
- [ ] Focused tests, `npm.cmd run verify`, required Rust tests, formatting,
      scoped strict lint, independent code/architecture review, clean commit,
      non-force push, remote SHA, PR/CI, product merge, install receipt, live
      Runtime Status and fresh-process replay all pass.

## Authorization

The user explicitly authorized this bounded implementation, non-force push,
PR, product-branch merge, deployment and installation. Public exposure,
credentials, security-control changes and destructive cleanup remain forbidden.
