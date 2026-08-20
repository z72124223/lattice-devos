---
ticket_id: TASK-079
title: Durable foreman state and takeover acceptance
spec_id: SPEC-006
spec_version: 2
module_id: foreman-state
constitution_version: 1.1
status: blocked
parallel_safe: false
depends_on:
  - TASK-048
  - TASK-049
  - TASK-078
branch: feature/task-079-durable-foreman-state
delivery_remote: origin
delivery_repository: github.com/z72124223/lattice-devos
delivery_push: local_only
delivery_archive: keep_open
allowed_paths:
  - PLANS.md
  - HANDOFF.md
  - Cargo.toml
  - Cargo.lock
  - crates/lattice-contracts/**
  - crates/lattice-foreman-state/**
  - crates/lattice-task-ledger/**
  - crates/lattice-ports/**
  - crates/lattice-orchestrator/**
  - crates/lattice-postgres-store/**
  - docs/adr/ADR-024-durable-foreman-state.md
  - docs/specs/SPEC-006-durable-foreman-state.md
  - docs/modules/foreman-state/MODULE_CONSTITUTION.md
  - docs/modules/task-ledger/MODULE_CONSTITUTION.md
  - docs/modules/postgres-store/MODULE_CONSTITUTION.md
  - docs/modules/lattice-ports/MODULE_CONSTITUTION.md
  - docs/modules/orchestrator-runtime/MODULE_CONSTITUTION.md
  - docs/tickets/TASK-079-durable-foreman-state.md
  - docs/reviews/**TASK_079**
  - tools/engineering-status-dashboard/branch-guide.zh-TW.json
---

# TASK-079 — Durable foreman state and takeover acceptance

## Objective

Implement the smallest versioned Ledger/port-owned foreman snapshot payload,
fresh-reader projection, and read-only dashboard watchdog described by SPEC-006.

## Acceptance criteria

- The six SPEC-006 acceptance criteria are covered by focused lightweight
  tests; no live PostgreSQL/TASK-051 gate is started.
- TASK-048 worker observations and TASK-049 closure remain read-only inputs.
- Snapshot persistence reaches only the approved Ledger/Postgres/port boundary
  and observes Writer Lease/fencing; no second truth is introduced.
- TASK-078 exporter files remain unchanged.

## Dependencies and overlap

TASK-048 `180a269` and TASK-049 `f03fcd8` are integrated as explicit merge
commits from clean feature tips. TASK-078 `5a5da01` is the clean base; its six
uncommitted source-worktree modifications are excluded. This ticket is not
parallel-safe because it changes the shared Ledger/Port/Postgres contracts.
TASK-084 is only the follow-up for epistemic learning/promotion; its
non-terminal worktree is not a prerequisite for this durable snapshot slice.

## TDD behaviors

1. RED/GREEN snapshot schema/generation and privacy rejection.
2. RED/GREEN fresh-reader active/blocked/next-action replay.
3. RED/GREEN watchdog stale/old-HEAD/duplicate/all-missed detection.
4. RED/GREEN dependency-blocked closure refusal and writer/fence substitution
   denial at the Ledger/port boundary.
5. RED/GREEN expiring epistemic pointer characterization: hypotheses remain
   non-authoritative and free-form hypothesis persistence rejects.

## Verification

```powershell
cargo test -p lattice-contracts --test worker_observation_contracts --locked
cargo test -p lattice-orchestrator --test window_closure --locked
npm.cmd run check
git diff --check
```

Focused TASK-079 test commands are added with the implementation. PostgreSQL
live acceptance and TASK-051 remain an explicit later gate.

## Human gate

Local work only. This ticket does not authorize push, merge, deployment,
release, archive, force, credential use, secret persistence, or live PostgreSQL
acceptance.

## Blocker

The pure schema/replay/watchdog foundation is implemented and tested, but the
existing Task Ledger has only `TASK` streams and closed events, while its
diagnostic field is explicitly non-authoritative. A valid production slice still
requires a versioned `FOREMAN_SNAPSHOT_RECORDED` event, fixed control-stream
identity, typed Port, PostgreSQL row/function/migration, and same-transaction
Writer Lease/fencing proof. Those changes are deliberately not substituted by
the new in-memory pure core and require the next bounded implementation ticket.
Full epistemic learning or promotion is additionally deferred to TASK-084.

The 2026-08-21 durable-binding audit found that a new global Store migration
after schema-v5 would invalidate the current Writer Lease v2 profile: its
extension identity and runtime binding admit only global schema 3 or 5. A
correct schema-v6 migration therefore requires a separately authorized Writer
Lease successor bridge plus Store catalog/ACL profile and revalidation. TASK-050
also has active uncommitted Ledger/Store governance edits. TASK-079 must not
modify either boundary, use diagnostics, or create an independent foreman table
to bypass this blocker.
