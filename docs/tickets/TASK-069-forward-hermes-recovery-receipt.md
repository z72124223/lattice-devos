---
ticket_id: TASK-069
spec_id: SPEC-002
spec_version: 31
module_id: hermes-adapter
constitution_version: 1.1
status: completed
parallel_safe: false
depends_on:
  - TASK-067
allowed_paths:
  - docs/tickets/TASK-069-forward-hermes-recovery-receipt.md
  - crates/lattice-hermes-adapter/src/production.rs
  - crates/lattice-hermes-adapter/tests/reflection_api.rs
  - PLANS.md
  - HANDOFF.md
branch: feature/task-051-p0-platform-live-acceptance
---

# TASK-069 Forward the Hermes recovery receipt

## Objective

Expose the existing secret-free `HermesRunRecoveryReceipt` through the
concrete `ProductionHermesPort` after a post-submit ambiguous failure, so a
same-process caller can attempt the already-governed reconciliation path
without resubmission.

## Acceptance Criteria

1. `ProductionHermesPort` forwards an immutable reference to the adapter's
   active recovery receipt and does not clone, persist, or mutate it.
2. The public production seam compiles against the existing opaque receipt
   type, and existing timeout/reconciliation tests retain their exact
   fail-closed behavior.
3. The receipt reveals only its existing run/request/session/input/model
   binding. No credential, raw output, environment value, or local path is
   added.
4. No automatic retry, resubmission, reconciliation, runtime launch/teardown,
   database write, MCP/CLI surface, dependency, or receipt schema is added.

The existing ambiguous-run/reconciliation tests prove the retained adapter
state and no-resubmission behavior. The production API seam test separately
freezes the exact borrowed `ProductionHermesPort` signature.

## Non-Goals

No durable recovery journal, cross-process replay, PostgreSQL schema, real
Hermes/provider/model, credential read, external network, public listener,
push, merge, deployment, payment, account change, or release.

## Completion Evidence

- RED: the production API seam failed to compile with `E0599` because
  `ProductionHermesPort::active_recovery_receipt` did not exist.
- GREEN: the API seam, same-process reconciliation, and timeout fail-closed
  tests each pass; the adapter all-target suite passes 81 tests with 9 explicit
  live-only ignores, plus 4 preparation tests.
- The implementation is one borrowed forwarding method. Independent review
  found no P0/P1 finding; its documentation now limits the receipt to a
  potential same-port reconciliation after post-submit ambiguity.
