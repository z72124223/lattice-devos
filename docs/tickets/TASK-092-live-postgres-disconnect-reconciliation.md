---
ticket_id: TASK-092
title: Live PostgreSQL disconnect reconciliation receipt
spec_id: SPEC-007
spec_version: 1
module_id: latticed
constitution_version: 2.5
status: in_progress
parallel_safe: false
depends_on:
  - TASK-090
  - TASK-091
execution_authorized_by: direct_user_goal_mode_authorization
execution_authorized_at_local: 2026-08-23
allowed_paths:
  - docs/tickets/TASK-092-live-postgres-disconnect-reconciliation.md
  - apps/lattice-runtime/src/bin/lattice-live-fault-acceptance.rs
  - apps/lattice-runtime/tests/**
  - crates/lattice-postgres-store/tests/postgres_live.rs
branch: product/lattice-control-mvp
---

# TASK-092 — Live PostgreSQL disconnect reconciliation receipt

## Objective

Add one bounded, Rust-only live acceptance phase that proves an owned
PostgreSQL operation loses its commit response, returns an unknown outcome,
and is reconciled by a fresh client without applying a second effect.

## Boundaries

- Reuse the fixed `CommitResponseDropProxy`; do not add a general proxy,
  shell, arbitrary target, external database, or process-kill capability.
- The runner accepts no caller database or fault parameters.
- Preserve the nonce-owned run root for any failed proof.

## Acceptance

1. The live receipt identifies the commit-response-loss marker and the
   fresh-client reconciliation result.
2. The reconciled effect counter is exactly one.
3. A missing marker, unknown reconciliation, duplicate effect, or failed
   owned-cluster stop fails closed.
