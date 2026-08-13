---
ticket_id: TASK-070
spec_id: SPEC-002
spec_version: 31
module_id: hermes-adapter
constitution_version: 1.1
status: ready
parallel_safe: false
depends_on:
  - TASK-069
allowed_paths:
  - docs/tickets/TASK-070-reconcile-hermes-normalized-evidence.md
  - crates/lattice-hermes-adapter/src/lib.rs
  - crates/lattice-hermes-adapter/src/production.rs
  - crates/lattice-hermes-adapter/tests/reflection_api.rs
  - PLANS.md
  - HANDOFF.md
branch: feature/task-051-p0-platform-live-acceptance
---

# TASK-070 Reconcile normalized Hermes evidence

## Objective

Return the existing normalized `HermesReflectionEvidence` after a bound,
same-process recovery receipt resolves an ambiguous submitted run, without
issuing another submission.

## Acceptance Criteria

1. Adapter reconciliation returns the strict canonical reflection paired with
   `RuntimeKind::Live` evidence derived from the same output digest.
2. The production port preserves its existing prepare, proxy completion, and
   liveness gates around that normalized reconciliation.
3. The existing scripted ambiguity fixture performs exactly one run submission
   and proves recovered reflection/evidence input and output bindings.
4. Existing `reconcile_reflection` remains available and compatible. No
   receipt, error, MCP/CLI, database, dependency, or ownership schema changes.

## Non-Goals

No automatic retry from `FullChainHermes`, durable recovery journal,
cross-process replay, WSL/live acceptance, real provider/model, credential
read, external network, public listener, push, merge, deployment, payment,
account change, or release.
