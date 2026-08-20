---
ticket_id: TASK-040
title: Code Mode toolchain repair
spec_id: SPEC-002
spec_version: 26
module_id: codex-adapter
constitution_version: 1.1
status: in-progress
parallel_safe: false
depends_on:
  - TASK-032
allowed_paths:
  - apps/lattice-runtime/src/composition.rs
  - crates/lattice-codex-adapter/src/delivery.rs
  - crates/lattice-codex-adapter/src/identity.rs
  - crates/lattice-codex-adapter/src/lib.rs
  - crates/lattice-codex-adapter/src/process.rs
  - crates/lattice-codex-adapter/tests/delivery_port.rs
  - crates/lattice-codex-adapter/tests/process.rs
  - docs/tickets/TASK-040-code-mode-toolchain-repair.md
  - docs/workflow/WORKFLOW_LEDGER.md
  - scripts/run-lattice-delivery.ps1
  - PLANS.md
branch: feature/task-040-code-mode-toolchain-repair
---

# TASK-040 - Code Mode toolchain repair

## Objective

Repair the pinned Code Mode toolchain binding so the codex adapter can resolve
the expected sandbox resources and surface reliable terminal status for the
current feature branch.

## Acceptance Criteria

- [ ] The TASK-040 ticket is discoverable by the engineering status dashboard
  from `feature/task-040-code-mode-toolchain-repair`.
- [ ] The branch is no longer reported as `UNKNOWN` because of a missing ticket.
- [ ] Focused codex-adapter/runtime verification passes for the current repair.
- [ ] The ticket status remains honest about the work state; no completion is
  claimed without current evidence.

## Non-Goals

- Do not claim official live acceptance, publication, deployment, or merge.
- Do not expand the codex adapter into unrelated gateway, shell, or database
  authority.

## Verification

```powershell
npm.cmd test
```
