# ADR-001: Event Ledger Is the Single Control-Plane Truth

- Status: accepted for Phase 1
- Date: 2026-07-29
- Decision owner: user request plus conservative local design

## Context

The product principle requires one fact source. Keeping mutable task status in
one file/database and an independent audit stream in another would permit
partial failures and contradictory recovery.

## Decision

Use a per-task append-only, hash-chained Task Ledger:

- the immutable Task Spec is recorded once;
- approvals, policy decisions, state transitions, lease events, runtime
  outcomes, verification, review, and integration evidence are appended;
- each append requires an `expected_sequence`;
- each command carries a `command_id`; replaying it returns the prior receipt
  instead of executing twice;
- current status and approval projections are derived by deterministic replay;
- every event includes its predecessor hash so truncation/reordering/tampering
  can be detected;
- audit sanitization removes secret-bearing fields before persistence.

The Orchestrator is the only component allowed to append workflow events.
Policy, Runtime, Reviewers, Scope Check, Git adapters, and the OpenClaw adapter
return results but cannot mutate task state.

## Consequences

- Recovery and tests have one authoritative state source.
- Append failure before a side effect blocks that side effect.
- A side effect whose outcome cannot be appended leaves the task blocked for
  human reconciliation.
- The Phase 1 file ledger is single-host only; a distributed ledger or database
  is a later architecture decision.

