---
spec_id: SPEC-009
status: ready
version: 1
modules:
  - module_id: task-ledger
    constitution_version: 2.4
  - module_id: orchestrator-runtime
    constitution_version: 2.8
---

# Autonomy Lifecycle Receipt

## Objective

Add a Task-Ledger-owned, append-only receipt for a verified autonomy-lifecycle
transition. It makes later background execution restart-safe without giving
the ledger scheduling, process, model, or execution authority.

## Trust and ownership

Task Ledger alone constructs, hashes, orders, exports, and replays the receipt.
Orchestrator may supply a typed transition recommendation but cannot construct
receipt bytes or write a database. A persistence adapter supplies untrusted
rows and must replay them through the Ledger verifier. The user goal and
granted authority remain authoritative; every technical fact, including a
foreground observation, heartbeat, holder death, and worker report, requires
current evidence before it may be recorded as a verified transition.

## Receipt contract

The new closed event is `AUTONOMY_LIFECYCLE_RECEIPT_RECORDED`. Its versioned,
domain-separated subject binds stream identity, predecessor event digest,
source state, transition event, destination state, evidence snapshot digest,
reason code, command identity, and schema/hash versions. State, event, and
reason are closed typed values; arbitrary text or JSON cannot define a legal
transition.

The planner rejects a stale head, stream mismatch, illegal state transition,
missing evidence, changed command content, or receipt/hash substitution.
Exact retry is byte-identical. Fresh replay rejects duplicate, missing,
truncated, reordered, orphaned, or substituted lifecycle rows and never uses
process memory or dashboard state as truth.

## Explicit boundaries

This ticket adds only the pure Ledger contract, fixtures, and replay verifier.
It does not add a Postgres migration, runner, scheduler, daemon, model call,
Codex window control, generic MCP input, execution port, push, merge, deploy,
release, account/credential action, destructive cleanup, or security downgrade.
A later ticket may wire the verified receipt to the existing PostgreSQL owner
only after a dedicated migration and fresh-process acceptance plan.

## Acceptance criteria

- Each legal SPEC-008 transition can be planned, exported, replayed, and
  re-verified from untrusted rows.
- Illegal transitions, stale heads, changed command reuse, digest substitution,
  duplicates, truncation, reordering, and missing fresh evidence fail closed.
- `YIELD_TO_FOREGROUND`, `ORPHAN_SUSPECTED`, and `RECONCILE_REQUIRED` cannot
  become `CLAIMED` without the required new evidence/reconciliation transition.
- Existing TASK-050 autonomy receipts and all historical streams replay
  byte-identically.
