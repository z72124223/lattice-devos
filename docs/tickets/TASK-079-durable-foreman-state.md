---
ticket_id: TASK-079
title: Durable foreman state and takeover acceptance
spec_id: SPEC-006
spec_version: 3
module_id: foreman-state
constitution_version: 1.2
status: completed
parallel_safe: false
depends_on:
  - TASK-048
  - TASK-049
  - TASK-078
  - TASK-087
branch: feature/task-079-durable-foreman-state
delivery_remote: origin
delivery_repository: github.com/z72124223/lattice-devos
delivery_push: authorized_non_force_feature_branch
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
  - db/migrations/0007_foreman_coordination.sql
  - docs/adr/ADR-024-durable-foreman-state.md
  - docs/specs/SPEC-006-durable-foreman-state.md
  - docs/modules/foreman-state/MODULE_CONSTITUTION.md
  - docs/modules/task-ledger/MODULE_CONSTITUTION.md
  - docs/modules/postgres-store/MODULE_CONSTITUTION.md
  - docs/modules/postgres-writer-lease/MODULE_CONSTITUTION.md
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

- The SPEC-006 criteria are covered by focused tests and a separately owned,
  disposable PostgreSQL gate; no TASK-051 resource is reused or started.
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

## Current implementation evidence and blocker

The pure schema/watchdog, fixed Ledger stream/event and generation replay,
typed Port, Store adapter, fixed-scalar `0007` child row, privacy checks, and
same-transaction Writer assertion are implemented. Focused crate tests,
schema-v6 profile tests, strict scoped Clippy, formatting, governance, TASK-048
and TASK-049 regressions pass.

Integration remains fail-closed. A new characterization test proves that the
production migration runner has no distinct six-row `ExactV5Prefix` to
seven-row `ExactV6Full` transition. TASK-087 also freezes Writer-v3 SQL and an
offline state verifier but does not provide the Writer-owned administrative
v3 bridge/apply/rebind operation required to create `G5_M3_W3_BRIDGE` and
finish `G6_M3_W3_CURRENT`. Store is constitutionally forbidden to manufacture
that Writer state. The disposable PostgreSQL gate is therefore `NOT_RUN`; this
is an offline production-path blocker, not a resource-lock failure.

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
acceptance uses only a marker-owned disposable cluster after an exclusive
resource preflight. TASK-051 remains untouched.

## Human gate

Local implementation, disposable PostgreSQL acceptance, one logical commit,
and a non-force push of this feature branch are authorized. Default-branch
merge, deployment, release, archive, force, credential change, secret
persistence, and TASK-051 resource use remain unauthorized.

## Durable binding authority

TASK-087 commit `e13e6d8ffb0ffeb4ae1eea7e33f535d1848f7d0f` is integrated by
history-preserving merge and supplies the reviewed Writer-v3/schema-v6 bridge.
TASK-079 owns `0007`, the fixed `FOREMAN_COORDINATION` stream and
`FOREMAN_SNAPSHOT_RECORDED` event, typed Port, Ledger-bound Store scalars, and
same-transaction Writer Lease assertion. A child physical row is valid only
when its matching Ledger event and command are committed in the same
transaction; it is never an independent current-state table. Epistemic pointers
remain expiring evidence and cannot derive or override lifecycle state.

## 2026-08-25 reconciliation

The former Writer-v3 apply/rebind blocker was completed by TASK-094, and
TASK-105 then completed the durable foreman checkpoint plus fresh-process
restart replay on schema-v6. PR #20 merged that accepted lineage as commit
`89b2d00e6fbed728d8aac1054dbbca59a33896e8`.

Current `lattice_runtime_status` independently reports foreman replay
`VERIFIED`, checkpoint `AVAILABLE`, generation `4`, completed count `1`, and
next action `ALL_COMPLETED`. This closes TASK-079 without treating its earlier
`NOT_RUN` evidence as if it had passed at the time.
