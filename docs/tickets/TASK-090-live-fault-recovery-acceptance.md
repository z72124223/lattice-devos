---
ticket_id: TASK-090
title: Rust-only live fault recovery acceptance foundation
spec_id: SPEC-007
spec_version: 1
module_id: latticed
constitution_version: 2.5
status: paused
parallel_safe: false
depends_on:
  - TASK-050
evidence_subjects:
  - TASK-051
execution_authorized_by: direct_user_goal_mode_authorization
execution_authorized_at_local: 2026-08-23
allowed_paths:
  - docs/specs/SPEC-007-live-fault-recovery-acceptance.md
  - docs/tickets/TASK-090-live-fault-recovery-acceptance.md
  - apps/lattice-runtime/Cargo.toml
  - apps/lattice-runtime/src/bin/lattice-live-fault-acceptance.rs
  - apps/lattice-runtime/tests/live_fault_acceptance.rs
branch: product/lattice-control-mvp
---

# TASK-090 — Rust-only live fault recovery acceptance foundation

## Objective

Implement the safe, opt-in Rust entry for the first SPEC-007 vertical slice:
fresh owned PostgreSQL admission, fresh-client replay, physical restart,
fresh-client replay, and no duplicate controlled effect.  The runner must
fail closed before any resource mutation when its fixed PostgreSQL identity or
owned-root checks fail.

## Boundaries

- This task does not repair TASK-051 or alter its terminal `FAIL` evidence.
- No PowerShell, batch harness, arbitrary shell, global configuration, service,
  production database, credential, deployment, push, merge, or release action.
- The implementation may only add the bounded binary/test and their documented
  dependencies.  It must reuse existing typed runtime paths rather than add a
  second task store or alternate task truth.

## Acceptance

1. Non-live tests prove opt-in gating, exact executable identity rejection,
   marker/root containment, and failure to clean up an unproved process.
2. The binary refuses live execution unless its fixed opt-in flag is present.
3. On this machine, the first slice either records the full restart evidence
   described in SPEC-007 or preserves the run root with one bounded blocker.
4. The existing lattice-runtime focused tests and formatting pass.

## 2026-08-25 reconciliation

This ticket is paused and genuinely incomplete. The planned
`apps/lattice-runtime/tests/live_fault_acceptance.rs` suite is absent. The
current binary has argument/opt-in unit coverage, but its source still records
that marker-and-stop-proof cleanup must be added; there is no complete
negative matrix for executable identity, root containment, and unproved
cleanup.

Next action: add those fail-closed non-live regressions and the bounded cleanup
proof, then run the exact current schema-v6 live acceptance. Later TASK-091 or
historical TASK-092 runs do not substitute for TASK-090's missing evidence.
