---
ticket_id: TASK-068
spec_id: SPEC-002
spec_version: 31
module_id: latticed
constitution_version: 1.7
status: ready
parallel_safe: false
depends_on:
  - TASK-067
allowed_paths:
  - docs/tickets/TASK-068-canonical-hermes-postgres-restart-replay.md
  - apps/lattice-runtime/src/composition.rs
  - scripts/run-task019-postgres.ps1
  - PLANS.md
  - HANDOFF.md
branch: feature/task-051-p0-platform-live-acceptance
---

# TASK-068 Canonical Hermes PostgreSQL restart replay

## Objective

Prove an exact canonical Hermes reflection receipt persists through the
marker-owned loopback PostgreSQL restart harness and is replayed by a fresh
Status process without activating, researching through, or persisting with
Hermes again.

## Acceptance Criteria

1. The initial phase persists one deterministic, graph-bound canonical Hermes
   reflection through the existing production PostgreSQL owner.
2. The harness performs a real bounded PostgreSQL stop/start between phases;
   the restart phase is a fresh Rust test process.
3. Fresh Status loads an exactly equal receipt and performs zero Hermes ready,
   research, and persistence calls.
4. Existing database schema, MCP tools, public APIs, receipt bytes, and
   production call paths remain unchanged.
5. Only marker-owned loopback PostgreSQL is used. No provider credential,
   Hermes/model request, external network, push, merge, deployment, payment,
   account change, or release occurs.

## Non-Goals

No real Hermes/provider/model, public network, schema migration, MCP change,
public test-support seam, push, merge, deployment, payment, account change, or
release.
