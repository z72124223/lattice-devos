---
ticket_id: TASK-101
title: Evidence-bound autonomy cycle supervisor core
spec_id: SPEC-007
spec_version: 1
module_id: orchestrator-runtime
constitution_version: 2.7
status: ready
parallel_safe: true
depends_on: []
branch: feature/task-101-autonomy-cycle
delivery_push: local_only
delivery_archive: keep_open
delivery_remote: origin
delivery_repository: github.com/z72124223/lattice-devos
allowed_paths:
  - crates/lattice-orchestrator/src/autonomy_cycle.rs
  - crates/lattice-orchestrator/src/lib.rs
  - crates/lattice-orchestrator/tests/autonomy_cycle.rs
  - docs/specs/SPEC-007-autonomy-cycle-supervisor.md
  - docs/tickets/TASK-101-autonomy-cycle-supervisor.md
  - docs/tickets/TASK-099-continuation-receipt.md
  - docs/modules/orchestrator-runtime/MODULE_CONSTITUTION.md
  - tools/engineering-status-dashboard/branch-guide.zh-TW.json
  - PLANS.md
  - HANDOFF.md
---

# TASK-101 — Evidence-bound autonomy cycle supervisor core

## Objective

Implement the pure decision core defined by SPEC-007. The next slice may wire
this core to a LATTICE-owned durable scheduler only after this ticket's tests,
review, and architecture gates pass.

## Verification

- `cargo test -p lattice-orchestrator --test autonomy_cycle`
- `cargo fmt --check`
- `cargo clippy -p lattice-orchestrator --all-targets --all-features -- -D warnings`
- `cargo test -p lattice-orchestrator`
- `npm.cmd run check`
- `git diff --check`
