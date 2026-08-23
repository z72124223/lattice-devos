# ADR-027: Task Ledger Owns Autonomy Lifecycle Receipts

- Status: accepted for TASK-103 pure contract work
- Date: 2026-08-23
- Decision owner: current user direction
- Related: SPEC-008, SPEC-009, TASK-101, TASK-102, TASK-103, ADR-023

## Decision

Task Ledger receives a new closed lifecycle-receipt event and owns its
canonical subject, hashing, ordering, idempotency, export, and replay. The
receipt binds an already verified transition and evidence snapshot; it does not
grant a claim, refresh a lease, schedule execution, or authorize a model.

The new receipt remains separate from `AUTONOMY_RECEIPT_RECORDED`, whose
frozen TASK-050 semantics and bytes are preserved. The Ledger does not depend
on Orchestrator: shared lifecycle values are represented by Ledger-owned
closed wire scalars in this slice. A future adapter maps only verified values
at its injected boundary.

## Consequences

Restart-safe background work becomes possible only after a later persistence
and runner slice proves PostgreSQL round-trip, governed heartbeats,
interruption, orphan reconciliation, and foreground preemption. No public MCP
tool or scheduler is broadened here.
