---
ticket_id: TASK-086
spec_id: SPEC-002
spec_version: 27
module_id: lattice-core-bootstrap
constitution_version: 1.0
status: blocked
parallel_safe: true
depends_on:
  - TASK-041
  - TASK-042
allowed_paths:
  - docs/tickets/TASK-086-task-041-042-integration.md
  - docs/reviews/CODE_REVIEW_TASK_086_2026-08-21.md
  - docs/reviews/ARCHITECTURE_REVIEW_TASK_086_2026-08-21.md
  - docs/reviews/INTEGRATION_TASK_086_2026-08-21.md
branch: feature/task-086-task-041-042-integration
delivery_remote: origin
delivery_repository: github.com/z72124223/lattice-devos
delivery_push: authorized_non_force_feature_branch
delivery_archive: keep_open
---

# TASK-086: TASK-041 and TASK-042 Integration Revalidation

## Objective

Revalidate whether the TASK-041 Rust-CI blocker is removed after the completed
TASK-042 Hermes strict-Clippy cleanup, without modifying either source
worktree. Create an isolated, history-preserving integration result and record
the combined local acceptance evidence.

## Exact Sources And Integration

- TASK-041 source: `feature/task-041-rust-ci` at
  `e3b10b42a88fb7484fef1d4dc668b1ebdd40e9a0`; clean and equal to its origin
  tracking ref at revalidation time.
- TASK-042 source: `feature/task-042-hermes-strict-clippy` at
  `a41dc7c3d9d6440cc4df66007c92ce9eb30c8953`; clean and equal to its origin
  tracking ref at revalidation time.
- Historical `integration/task-041-task-042` was local-only at `f4c2dbe`, had
  no origin tracking ref, and was not an ancestor of either terminal source;
  it was retained untouched as historical evidence.
- New integration worktree: `lattice-worktrees/task-086-task-041-042-integration`.
- Integration commit: `5b59bf4414889d2c674a934ccf32e9887da26883`, a non-conflicting
  merge whose parents are exactly the two source commits above.

## Result

TASK-042 removes the eleven Hermes strict-Clippy errors that had blocked
TASK-041. The combined workspace strict-Clippy command now fails solely on the
pre-existing, out-of-scope `clippy::manual_inspect` at
`apps/lattice-runtime/src/composition.rs:2720`. No Hermes error is emitted.

Therefore TASK-041 is no longer blocked by Hermes, but the Rust-CI acceptance
gate remains `BLOCKED` until the runtime owner resolves that separate lint. No
shared contract or runtime file was changed to hide the failure.

## Required Follow-up

Have the owner of `apps/lattice-runtime/src/composition.rs` resolve the strict
Clippy finding in a separately scoped ticket, then refresh this integration
branch and rerun the complete matrix. This ticket stays open and is not a
merge, release, deployment, or remote-Actions-success claim.
