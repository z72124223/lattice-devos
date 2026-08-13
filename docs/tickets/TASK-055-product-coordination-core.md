---
ticket_id: TASK-055
title: Product coordination decision core
spec_id: SPEC-002
spec_version: 28
module_id: orchestrator-runtime
constitution_version: 2.5
status: completed
parallel_safe: false
depends_on:
  - TASK-050
  - TASK-054
allowed_paths:
  - docs/tickets/TASK-055-product-coordination-core.md
  - docs/specs/SPEC-002-autonomous-development-platform.md
  - docs/modules/orchestrator-runtime/MODULE_CONSTITUTION.md
  - crates/lattice-orchestrator/src/coordination.rs
  - crates/lattice-orchestrator/src/lib.rs
  - crates/lattice-orchestrator/tests/coordination_control.rs
  - apps/lattice-runtime/src/coordination.rs
  - apps/lattice-runtime/src/lib.rs
  - apps/lattice-runtime/tests/coordination.rs
  - PLANS.md
  - HANDOFF.md
branch: feature/task-051-p0-platform-live-acceptance
---

# TASK-055 Product coordination decision core

## Objective

Make the verified offline coordination rules a normal LATTICE product API. The
core projects work/evidence state, admits a dispatch round only from verified
inputs, recomputes the next round after completion registration, and recommends
archival for completed work with no unfinished dependent work.

## Acceptance criteria

1. Only a unique `READY` work item with declared non-empty unique resources,
   declared dependencies, and `VERIFIED DONE` evidence for every dependency is
   dispatchable.
2. `UNKNOWN`, `BLOCKED`, incomplete evidence, duplicate work/report IDs,
   undeclared/duplicate/self dependencies, and resource conflicts fail closed.
3. Verified completion makes the next eligible dependent dispatchable;
   completed work with no unfinished dependent receives `ARCHIVE`, otherwise
   `RETAIN`.
4. `lattice-runtime` exposes the typed gate without adding MCP, window/process
   control, arbitrary file/network/credential/database access, deployment, or
   self-modification. Dispatch output may enter only the existing governed
   LATTICE execution path.
5. Focused, affected integration, full available Rust, formatting/static, and
   project checks pass, or an unrelated pre-existing failure is recorded.

## Completion evidence

- `cargo test -p lattice-orchestrator --all-targets`,
  `cargo test -p lattice-runtime --all-targets`, and
  `cargo test --workspace` passed on 2026-08-13.
- `cargo clippy -p lattice-orchestrator --all-targets --all-features -- -D warnings`,
  `cargo fmt --all -- --check`, `npm.cmd run check`, and `npm.cmd test` passed.
- Full workspace/runtime strict Clippy remains blocked only by pre-existing
  diagnostics in unchanged runtime and Hermes files; TASK-055 adds no lint
  suppression.
- Read-only code/architecture review found no TASK-055 defect; independent
  reviewer separation is not proven. No dependency, MCP, database, process,
  filesystem, network, credential, deployment, or release surface was added.
