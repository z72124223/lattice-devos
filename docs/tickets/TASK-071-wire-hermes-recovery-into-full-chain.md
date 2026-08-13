---
ticket_id: TASK-071
spec_id: SPEC-002
spec_version: 31
module_id: latticed
constitution_version: 1.7
status: ready
parallel_safe: false
depends_on:
  - TASK-070
allowed_paths:
  - docs/tickets/TASK-071-wire-hermes-recovery-into-full-chain.md
  - apps/lattice-runtime/src/composition.rs
  - PLANS.md
  - HANDOFF.md
branch: feature/task-051-p0-platform-live-acceptance
---

# TASK-071 Wire Hermes recovery into FullChainHermes

## Objective

Use the active, secret-free recovery receipt on the already-bound production
port to reconcile one post-submit ambiguity inside `FullChainHermes`, without
issuing another run submission or recomputing normalized evidence.

## Acceptance Criteria

1. A successful first run returns unchanged output and never inspects a
   recovery receipt.
2. A failed first run with no active receipt returns the exact original
   failure and does not reconcile.
3. A failed first run with an active receipt performs exactly one same-port
   reconciliation, returns its normalized evidence unchanged, and never calls
   the run path again.
4. Production uses `ProductionHermesPort::active_recovery_receipt` and
   `reconcile_reflection_evidence`; candidate validation and later persistence
   order remain unchanged.
5. No public API, trait, receipt/error schema, database, MCP/CLI, dependency,
   credential, network, model, or ownership change is introduced.

## Non-Goals

No recovery loop, cross-process recovery, durable journal, failure
persistence, WSL/live acceptance, real provider/model, credential read,
external network, public listener, push, merge, deployment, payment, account
change, or release.
