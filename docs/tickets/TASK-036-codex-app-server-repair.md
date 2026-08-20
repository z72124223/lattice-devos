---
ticket_id: TASK-036
spec_id: SPEC-002
spec_version: 27
module_id: codex-adapter
constitution_version: 1.1
additional_modules:
  - module_id: lattice-contracts
    constitution_version: 1.11
  - module_id: lattice-ports
    constitution_version: 1.7
  - module_id: orchestrator-runtime
    constitution_version: 2.2
  - module_id: latticed
    constitution_version: 1.1
  - module_id: postgres-store
    constitution_version: 1.4
status: partial
delivery_remote: origin
delivery_repository: github.com/z72124223/lattice-devos
delivery_push: authorized_non_force_feature_branch
delivery_archive: keep_open
parallel_safe: false
depends_on:
  - TASK-032
  - TASK-033
allowed_paths:
  - HANDOFF.md
  - PLANS.md
  - docs/tickets/TASK-036-codex-app-server-repair.md
  - docs/workflow/WORKFLOW_LEDGER.md
  - docs/reviews/WORKFLOW_AUDIT_TASK_036_2026-08-20.md
  - docs/reviews/CODE_REVIEW_TASK_036_2026-08-20.md
  - docs/reviews/ARCHITECTURE_REVIEW_TASK_036_2026-08-20.md
  - docs/reviews/INTEGRATION_TASK_036_2026-08-20.md
branch: feature/task-036-codex-app-server-repair
---

## Objective

Repair the official Codex app-server delivery checkpoint so the engineering
status projection no longer falls back to `UNKNOWN` when the branch is present
but the corresponding task record is missing. Preserve the current verified
behavioral evidence: the official live attempt remains `NEEDS_REVIEW`, the
consumed one-shot is not retried, and the status view must derive its terminal
state from the authoritative ticket plus handoff evidence.

## Acceptance Criteria

- [ ] The branch has one authoritative `TASK-036` record with a non-`UNKNOWN`
  status that matches the current evidence truth.
- [ ] The repair does not claim TASK-032 completion, because official live
  acceptance remains `NEEDS_REVIEW`.
- [ ] The engineering-status projection can resolve this branch to the task
  record without inventing a ticket or terminal state.
- [ ] Focused verification confirms the ticket/status projection path with the
  current handoff and workflow ledger evidence.

## Non-Goals

- Do not rerun the consumed official Codex live attempt.
- Do not change app-server behavior, delivery protocol semantics, or
  PostgreSQL truth beyond status repair evidence.
- Do not merge, publish, deploy, or mark the branch complete.

## Verification

- `git status --short --branch`
- `rg -n "TASK-036|NEEDS_REVIEW|UNKNOWN" docs HANDOFF.md PLANS.md`
- any focused status-projection test or generator check needed to prove the
  dashboard no longer reports `UNKNOWN`
