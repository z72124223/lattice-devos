---
ticket_id: TASK-068
spec_id: SPEC-002
spec_version: 31
module_id: latticed
constitution_version: 1.7
status: partial
parallel_safe: false
depends_on:
  - TASK-067
allowed_paths:
  - docs/tickets/TASK-068-canonical-hermes-postgres-restart-replay.md
  - docs/tickets/TASK-069-forward-hermes-recovery-receipt.md
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

## Partial Evidence And Integration Blocker

- RED: the PostgreSQL harness initially rejected the absent
  `-RunTask068HermesReplayGate` switch before starting a service.
- GREEN: the marker-owned loopback harness completed initial persistence, a
  physical PostgreSQL stop/start, and a fresh-process Status replay with the
  same receipt digest. Effect counts were `1/1/1` for initial
  ready/research/persist and `0/0/0` after restart.
- The complete non-`MemoryOnly` Store initial/restart profile also passes on
  the current contract-conflicted five-entry autonomy baseline after
  correcting stale test expectations. This does not prove the approved
  Registry-plus-autonomy combined profile.
- Integration remains partial because the inherited branch assigns autonomy
  receipt migration ordinal `0005`, while accepted ADR-020 reserves exact
  `0005` for Project Registry. The completed Registry source commits
  `12f7100`/`a1aced9` are not ancestors of this worktree; autonomy commit
  `714f3b9` occupies the ordinal here. Commits `aa097e0` and `593112e` only
  align tests/admission with that inherited five-entry profile, while
  `3ac4b1c` proves the replay mechanics.

### Blocked follow-up — POSTGRES-REGISTRY-AUTONOMY-MIGRATION-RECONCILIATION

The stop boundary is explicit: this ticket does not cherry-pick or merge
TASK-022, reorder a migration, change schema/manifest contracts, or mark
TASK-068 completed. A separately authorized follow-up must preserve Registry
`0005` and move autonomy to a later ordinal/profile before combined product
integration can be claimed.
