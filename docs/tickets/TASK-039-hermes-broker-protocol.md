---
ticket_id: TASK-039
spec_id: SPEC-002
spec_version: 27
module_id: hermes-adapter
constitution_version: 1.0
additional_modules:
  - module_id: lattice-contracts
    constitution_version: 1.11
  - module_id: lattice-ports
    constitution_version: 1.7
  - module_id: orchestrator-runtime
    constitution_version: 2.2
  - module_id: latticed
    constitution_version: 1.1
  - module_id: codex-adapter
    constitution_version: 1.1
status: completed
parallel_safe: false
depends_on:
  - TASK-032
  - TASK-033
allowed_paths:
  - README.md
  - PLANS.md
  - HANDOFF.md
  - Cargo.toml
  - Cargo.lock
  - apps/lattice-runtime/**
  - crates/lattice-contracts/**
  - crates/lattice-ports/**
  - crates/lattice-orchestrator/**
  - crates/lattice-hermes-adapter/**
  - crates/lattice-codex-adapter/**
  - docs/specs/SPEC-002-autonomous-development-platform.md
  - docs/adr/ADR-006-single-codex-owner-and-read-only-agents.md
  - docs/modules/hermes-adapter/**
  - docs/modules/codex-adapter/**
  - docs/modules/lattice-contracts/**
  - docs/modules/lattice-ports/**
  - docs/modules/orchestrator-runtime/**
  - docs/modules/latticed/**
  - docs/tickets/TASK-039-hermes-broker-protocol.md
  - docs/workflow/WORKFLOW_LEDGER.md
  - docs/reviews/WORKFLOW_AUDIT_TASK_039_2026-08-20.md
  - docs/reviews/CODE_REVIEW_TASK_039_2026-08-20.md
  - docs/reviews/ARCHITECTURE_REVIEW_TASK_039_2026-08-20.md
  - docs/reviews/INTEGRATION_TASK_039_2026-08-20.md
branch: feature/task-039-hermes-broker-protocol
---

## Objective

Confirm the Hermes broker protocol is already fully implemented on this branch
and repair the authoritative task/status evidence so governance tools project
the current terminal state instead of UNKNOWN. The owned scope is the Hermes
adapter boundary and its status evidence only; no new broker feature work is
required in this window.

## Acceptance Criteria

- [x] The Hermes adapter owns a read-only, bounded research/reflection lane
  with a dedicated profile, isolated candidate output, and fail-closed
  provenance checks.
- [x] The production runner binds exact executable identity, profile/capability
  evidence, and protocol framing for the broker lane.
- [x] The broker protocol rejects malformed, ambiguous, timeout, or
  cancellation-uncertain results instead of reporting success.
- [x] Focused Hermes adapter tests and relevant workspace verification pass on
  the current branch.
- [x] The governance files name the task, constitution, branch, and current
  marker so the status pipeline can resolve the current task.

## Non-Goals

- Add a writable Hermes product writer.
- Reopen TASK-032 or TASK-033 implementation work.
- Push, merge, deploy, or publish.

## Human Gate

None for this local status repair. Any future protocol expansion requires the
normal versioned constitution and architecture review process.
