---
ticket_id: TASK-106
title: Durable dependency suspension and safe continuation
spec_id: SPEC-010
spec_version: 1
module_id: latticed
constitution_version: 3.2
additional_modules:
  - module_id: foreman-state
    constitution_version: 1.4
  - module_id: lattice-ports
    constitution_version: 2.2
  - module_id: postgres-store
    constitution_version: 1.20
  - module_id: workspace-git
    constitution_version: 1.1
status: in_progress
parallel_safe: false
depends_on: [TASK-105]
branch: feature/task-106-dependency-continuation
implementation_worktree: lattice-worktrees/task-106-dependency-continuation
implementation_base: d248200ebd1c7958a7e0dc40ec697d918f9e3d39
delivery_remote: origin
delivery_repository: github.com/z72124223/lattice-devos
delivery_push: authorized_non_force
delivery_merge: authorized_product_branch
delivery_deploy_install: authorized_no_app_restart
allowed_paths:
  - Cargo.lock
  - PLANS.md
  - package.json
  - apps/lattice-runtime/**
  - crates/lattice-foreman-state/**
  - crates/lattice-ports/**
  - crates/lattice-postgres-store/**
  - crates/lattice-orchestrator/tests/foreman_checkpoint.rs
  - src/workspace/git-workspace.js
  - test/git-workspace.integration.test.js
  - scripts/lattice-dependency-worktree.mjs
  - scripts/start-lattice-runtime-postgres.ps1
  - scripts/test-task106-dependency-continuation.ps1
  - docs/specs/SPEC-010-dependency-continuation.md
  - docs/tickets/TASK-106-dependency-continuation.md
  - docs/modules/foreman-state/MODULE_CONSTITUTION.md
  - docs/modules/lattice-ports/MODULE_CONSTITUTION.md
  - docs/modules/latticed/MODULE_CONSTITUTION.md
  - docs/modules/postgres-store/MODULE_CONSTITUTION.md
  - docs/modules/workspace-git/MODULE_CONSTITUTION.md
  - docs/reviews/WORKFLOW_LEDGER_TASK_106_2026-08-25.md
---

# TASK-106 — Durable dependency suspension and safe continuation

## Objective

Deliver the one vertical behavior in SPEC-010 while retaining PostgreSQL as
durable truth, Git as live integration evidence, and exactly one writable
engineering task.

## TDD behaviors

1. Closed dependency binding and replay projection.
2. Safe owned child creation and fail-closed Git reconciliation.
3. MCP parsing plus exact-retry-before-Git ordering.
4. PostgreSQL multi-process `BLOCKED` and `RESUMED` replay.

## Completion gate

This ticket remains `in_progress` until current tests and independent review,
clean commit, non-force push, remote SHA, PR/CI, product merge, deployment and
installation receipt, live Runtime Status and fresh-process PostgreSQL replay
all succeed. A feature branch, local pass, PR, or green CI is intermediate.

## Safety

No force push, reset/clean, unknown-worktree mutation, destructive deletion,
public-network exposure, credential/account change, security-control reduction,
or Codex App restart is allowed.
