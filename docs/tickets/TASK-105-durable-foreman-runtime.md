---
ticket_id: TASK-105
title: Durable foreman runtime checkpoint and restart replay
spec_id: SPEC-009
spec_version: 1
module_id: latticed
constitution_version: 2.7
additional_modules:
  - module_id: orchestrator-runtime
    constitution_version: 2.7
  - module_id: lattice-ports
    constitution_version: 2.1
  - module_id: foreman-state
    constitution_version: 1.3
  - module_id: task-ledger
    constitution_version: 2.7
  - module_id: postgres-store
    constitution_version: 1.15
status: in_progress
parallel_safe: false
depends_on: [TASK-094]
branch: feature/task-105-durable-foreman-runtime
implementation_worktree: lattice-worktrees/task-105-durable-foreman-runtime
implementation_base: 387f556a5b17adb75274c1387cf517654650c90b
integration_input: 1e4ac5dddad71648ed041d3bd1839f70aefdf393
delivery_remote: origin
delivery_repository: github.com/z72124223/lattice-devos
delivery_push: authorized_non_force_feature_branch
delivery_archive: keep_open
allowed_paths:
  - Cargo.lock
  - PLANS.md
  - HANDOFF.md
  - apps/lattice-runtime/**
  - crates/lattice-foreman-state/**
  - crates/lattice-orchestrator/**
  - crates/lattice-ports/**
  - crates/lattice-task-ledger/**
  - crates/lattice-postgres-store/**
  - crates/lattice-postgres-writer-lease/**
  - docs/adr/ADR-027-durable-foreman-runtime-boundary.md
  - docs/specs/SPEC-009-durable-foreman-runtime.md
  - docs/tickets/TASK-105-durable-foreman-runtime.md
  - docs/modules/foreman-state/MODULE_CONSTITUTION.md
  - docs/modules/task-ledger/MODULE_CONSTITUTION.md
  - docs/modules/lattice-ports/MODULE_CONSTITUTION.md
  - docs/modules/orchestrator-runtime/MODULE_CONSTITUTION.md
  - docs/modules/latticed/MODULE_CONSTITUTION.md
  - docs/modules/postgres-store/MODULE_CONSTITUTION.md
  - docs/reviews/WORKFLOW_LEDGER_TASK_105_2026-08-25.md
  - scripts/start-lattice-runtime-postgres.ps1
  - scripts/test-task105-durable-foreman-runtime.ps1
---

# TASK-105 — Durable foreman runtime checkpoint and restart replay

## Objective

Integrate exact TASK-094 into the product base and deliver the one vertical
runtime path in SPEC-009 without widening product scope.

## Dependencies and overlap

TASK-094 exact remote SHA is integrated by merge commit `d116e423`. This ticket
is the sole writable task because MCP, runtime composition, Writer authority and
schema-v6 startup share the same files and durable state.

## TDD behaviors

1. Exact-next generation and replay-first checkpoint semantics.
2. Exact-seven canonical MCP schema with prohibited-field rejection.
3. Orchestrated Writer acquire/append/release failure ordering.
4. Explicit bootstrap matrix and verified fresh runtime clients.
5. Marker-owned PostgreSQL fresh-process checkpoint/restart replay.

## Acceptance criteria

- [ ] Every SPEC-009 acceptance criterion has current evidence.
- [ ] Focused, integration, Control regression, format/check and scoped strict
      lint pass; unrelated workspace failures are recorded without relabeling.
- [ ] Clean local commits only; no worker push, PR, product merge, deploy,
      install, service/database mutation or archive-ready claim.

## Human gate

Parent foreman owns independent review, non-force push, product merge, install,
deployment and live post-deploy revalidation.
