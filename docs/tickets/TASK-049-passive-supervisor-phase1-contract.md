---
ticket_id: TASK-049
spec_id: GITHUB-ISSUE-002
spec_version: 2026-08-09
module_id: passive-supervisor
constitution_version: 0.1
status: blocked-upstream-contract
parallel_safe: true
depends_on:
  - TASK-038
  - TASK-039
allowed_paths:
  - contracts/passive-supervisor-phase1/**
  - docs/modules/passive-supervisor/**
  - docs/tickets/TASK-049-passive-supervisor-phase1-contract.md
branch: feature/issue-2-passive-supervisor-20260810
---

## Objective

Deliver the isolated, executable contract checkpoint for GitHub Issue #2 Phase
1. The checkpoint classifies authoritative evidence as pending, running,
terminal, or stalled and recommends only the next passive observation. It must
not register a production crate, compose a runtime, write PostgreSQL, dispatch
work, supervise a process, activate a daemon, or duplicate TASK-038/TASK-039.

## Dependency Gate

Production readiness is false until both upstream owners publish an exact
stable checkpoint:

| Dependency | Required machine evidence | Current blocker |
|---|---|---|
| TASK-038 / Issue #4 | stable branch, 40-hex commit, contract version, and 64-hex digest of formal `latticed` `lattice_task_submit/status` verification | `SUPERVISOR_TASK_CONTROL_CHECKPOINT_MISSING` |
| TASK-039 / Issue #6 | stable branch, 40-hex commit, contract version, and 64-hex digest of provider-neutral worker/session verification | `SUPERVISOR_WORKER_OBSERVATION_CHECKPOINT_MISSING` |

The checked-in dependency gate must remain RED while either blocker exists.
Passing pure classifier tests does not satisfy this gate and cannot be reported
as product completion.

## Acceptance Criteria

- [x] A standard-library-only contract evaluates immutable snapshots with an
  explicit observation clock and no I/O or hidden time.
- [x] Named event fixtures cover no work, pending before dispatch, running with
  fresh bound activity, terminal with verification, and non-terminal stalled
  evidence.
- [x] Fault fixtures reject cross-task evidence, future observations,
  impossible ordering, missing task truth, and terminal conflicts.
- [x] `STALLED` remains a passive evidence classification and never mutates or
  substitutes for the Task Domain state.
- [x] Recommendations are limited to wait/observe/read/recommend-persist/sleep;
  no dispatch, scheduling, retry, repair, lease, process, Git, database, or
  service action is representable.
- [x] Dependency readiness produces exact stable blocker codes and the current
  upstream fixture fails the dedicated gate test.
- [x] Root `Cargo.toml`, runtime composition, PostgreSQL schema/migrations, and
  daemon/service entrypoints remain byte-identical to base `845328d`.

## Verification

- `cargo test --manifest-path contracts/passive-supervisor-phase1/Cargo.toml --lib --test passive_classifier`
- `cargo test --manifest-path contracts/passive-supervisor-phase1/Cargo.toml --test stable_dependency_gate` (expected RED until both upstream checkpoints are stable)
- module-constitution validator for `passive-supervisor`
- `git diff --check` and exact forbidden-path comparison against `845328d`

## Non-Goals

- Deterministic dispatch, AI-assisted scheduling, automatic repair, retry,
  cancellation, lease revocation, service activation, or persistent daemon.
- TASK-038 task-control implementation, Issue #6 worker/session observation,
  root workspace membership, PostgreSQL schema/migration, MCP tools, runtime
  composition, DB/WSL/live tests, full workspace test matrix, push, merge,
  deployment, or service creation.

## Completion Rule

This ticket may checkpoint the pure classifier and its expected RED dependency
gate. It remains `BLOCKED` for production integration until both upstream
checkpoints are exact and the gate test turns green without fixtures or mocks
claiming unavailable success.
