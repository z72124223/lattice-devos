---
spec_id: SPEC-008
status: ready
version: 1
modules:
  - module_id: orchestrator-runtime
    constitution_version: 2.8
---

# Durable Autonomy Lifecycle Gate

## Objective

Define the closed in-memory lifecycle used by a future LATTICE-owned autonomy
runner. The gate makes foreground work preempt autonomous work and prevents an
unknown interrupt, lost heartbeat, or orphaned holder from becoming a resume.
It is a pure decision boundary; it is not a scheduler, daemon, model caller,
or durable record.

## Closed states and events

States are `IDLE`, `CLAIMED`, `HEARTBEAT_DUE`, `YIELD_TO_FOREGROUND`,
`INTERRUPTING`, `ORPHAN_SUSPECTED`, and `RECONCILE_REQUIRED`.

Only these transitions are legal:

- `IDLE + CLAIM_VERIFIED -> CLAIMED`.
- `CLAIMED + HEARTBEAT_DUE -> HEARTBEAT_DUE`.
- `HEARTBEAT_DUE + HEARTBEAT_VERIFIED -> CLAIMED`.
- `CLAIMED|HEARTBEAT_DUE + FOREGROUND_TASK_OBSERVED -> YIELD_TO_FOREGROUND`.
- `CLAIMED + INTERRUPT_REQUESTED -> INTERRUPTING`.
- `INTERRUPTING + INTERRUPT_OUTCOME_UNKNOWN -> RECONCILE_REQUIRED`.
- `HEARTBEAT_DUE + HOLDER_DEATH_VERIFIED -> ORPHAN_SUSPECTED`.
- `ORPHAN_SUSPECTED + NEW_EVIDENCE_SNAPSHOT -> RECONCILE_REQUIRED`.
- `YIELD_TO_FOREGROUND + NEW_EVIDENCE_SNAPSHOT -> IDLE`.
- `RECONCILE_REQUIRED + RECONCILIATION_VERIFIED -> IDLE`.

Every other pair is rejected. In particular, orphaned, yielded, and
reconciliation-required work can never claim or resume directly.

## Acceptance criteria

- Foreground observation wins over background selection and requires a fresh
  evidence snapshot before another cycle.
- Unknown interrupt outcome and holder death fail closed into reconciliation.
- The implementation is pure Rust with no process, timer, database, model,
  filesystem, network, MCP, or scheduler dependency.
