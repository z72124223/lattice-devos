---
ticket_id: TASK-071
spec_id: SPEC-002
spec_version: 31
module_id: latticed
constitution_version: 1.7
status: completed
parallel_safe: false
depends_on:
  - TASK-070
allowed_paths:
  - docs/tickets/TASK-071-wire-hermes-recovery-into-full-chain.md
  - docs/tickets/TASK-072-production-hermes-recovery-acceptance.md
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
2. A first failure is eligible for automatic reconciliation only when it is
   exact `HERMES_LOOPBACK_TIMEOUT`, `HERMES_RUN_DEADLINE_EXCEEDED`, or
   `HERMES_LOOPBACK_TRANSPORT_FAILED` and the active receipt contains a known
   run ID. Every other failure, or a receipt without a run ID, returns the
   exact initial failure and does not reconcile.
3. An eligible first failure performs exactly one same-port reconciliation,
   returns successful normalized evidence unchanged, and never calls the run
   path again. A second eligible observation failure becomes the existing
   `LATTICE_DELIVERY_RECONCILIATION_REQUIRED`; a definitive reconciliation
   failure remains exact.
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

## Completion Evidence

- RED: structural failures with active receipts reached reconciliation and
  panicked the fail-closed regression; after the eligibility gate, a repeated
  transport failure still returned `Unavailable`, and the canonical wrapper
  collapsed reconciliation-required state to `HermesExecution`.
- GREEN: `f7cd1b3` wires the existing production receipt and normalized
  reconciliation APIs; `fbbebbb` restricts entry to the three exact transient
  codes plus a known run ID; `27a7b7f` preserves repeated uncertainty as the
  existing reconciliation-required result without changing public error
  schemas.
- Runtime all-target verification passes 97 tests with one explicit
  marker-owned PostgreSQL live-only ignore; the 20 composition, 1 coordination,
  5 dispatch, 31 MCP, and 1 task-control integration tests all pass. Project
  verification passes 48 Node tests; format and diff checks pass.
- Three independent final reviews report no P0/P1 correctness or architecture
  findings. No WSL, real provider/model, credential read, external network,
  database mutation, push, merge, deployment, payment, account change, or
  release occurred.
