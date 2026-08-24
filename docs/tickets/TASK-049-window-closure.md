---
ticket_id: TASK-049
title: Execution window closure eligibility
spec_id: AUTONOMOUS_EXECUTION_CONTROL
spec_version: 1
module_id: orchestrator-runtime
constitution_version: 2.4
status: completed
parallel_safe: false
depends_on:
  - commit:175633ca40352a314a0b699c7cb53697c239d481
branch: feature/task-049-window-closure
delivery_remote: origin
delivery_repository: github.com/z72124223/lattice-devos
delivery_push: authorized_non_force_feature_branch
delivery_archive: after_success
allowed_paths:
  - crates/lattice-orchestrator/src/lib.rs
  - crates/lattice-orchestrator/src/window_closure.rs
  - crates/lattice-orchestrator/tests/window_closure.rs
  - docs/tickets/TASK-049-window-closure.md
  - PLANS.md
---

# TASK-049 — Execution window closure eligibility

## Objective

Provide one pure, fail-closed decision for whether an execution window may be
archived by its coordinator after a durable handoff is recorded. This ticket
does not persist handoffs or call a Codex platform archive capability.

## Acceptance criteria

- [x] Only execution windows in `Completed` or genuinely `Blocked` Task Domain
      state can become archive-eligible.
- [x] Eligibility requires all durable continuation fields: scope, terminal
      status, verified work, remaining risks, next step, and a related change
      reference when one exists.
- [x] Missing or partial handoffs, non-terminal states, failures,
      cancellations, planning, coordination, and conversation windows remain
      open with a typed reason.
- [x] The Orchestrator remains pure and gains no persistence, platform archive,
      filesystem, process, database, or transport dependency.

## Verification

`cargo test -p lattice-orchestrator --test window_closure --locked` verifies
the complete handoff, incomplete/missing handoff, non-terminal execution, and
non-execution classifications. Repository governance is checked with
`npm.cmd run check`.

## Delivery boundary

This completed local implementation is authorized only for a non-force push of
this named feature branch. It does not authorize a pull request, default-branch
merge, deployment, release, or platform archive on a failed delivery.

CURRENT TASK-049 — terminal Window Closure evidence is complete; delivery must
use the fail-closed TASK-078 finisher before the coordinator may archive this
execution window.
