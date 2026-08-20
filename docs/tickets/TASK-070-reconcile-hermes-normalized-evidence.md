---
ticket_id: TASK-070
spec_id: SPEC-002
spec_version: 31
module_id: hermes-adapter
constitution_version: 1.1
status: completed
parallel_safe: false
depends_on:
  - TASK-069
allowed_paths:
  - docs/tickets/TASK-070-reconcile-hermes-normalized-evidence.md
  - docs/tickets/TASK-071-wire-hermes-recovery-into-full-chain.md
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

## Completion Evidence

- RED: both adapter and production-port `reconcile_reflection_evidence`
  references failed to compile with `E0599`.
- GREEN: the scripted post-submit ambiguity performs exactly one run POST,
  reconciles through status, and returns exact invocation/input/output binding
  with `RuntimeKind::Live`. The production seam preserves its existing
  prepare, proxy-completion, and liveness gates.
- Adapter all-target tests pass 81 tests with 9 explicit live-only ignores,
  plus 4 preparation tests. Format, diff, Node verification, and independent
  architecture/code reviews pass with no P0/P1/P2.
