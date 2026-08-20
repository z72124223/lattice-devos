---
ticket_id: TASK-034
title: Hermes reflection containment acceptance
module_id: hermes-adapter
status: failed
parallel_safe: false
depends_on: []
branch: feature/task-034-hermes-reflection
delivery_remote: origin
delivery_repository: github.com/z72124223/lattice-devos
delivery_push: authorized_non_force_feature_branch
delivery_archive: keep_open
allowed_paths:
  - docs/tickets/TASK-034-hermes-reflection.md
---

# TASK-034 — Hermes reflection containment acceptance

## Objective

Verify the pinned, read-only Hermes reflection adapter and record a terminal
result without treating scripted tests as proof of whole-process containment.

## Result

**FAILED — current-machine containment acceptance is blocked.**

The exact WSL/bubblewrap socketpair canary was executed on 2026-08-21 and
failed closed with `HERMES_SOCKETPAIR_CANARY_BWRAP_REJECTED`. The host's
`/usr/bin/bwrap` SHA-256 was
`0abea81db798ebf6b4742ac0664802d97521547a353c2a0dbdc21d76cbbfd2c0`,
which does not match this adapter's pinned containment identity
`8e19e40e7d5f7a7e8b488c7926feb040eab6ed10c58fa360e266d2f70670e92b`.

## Evidence

- `cargo test -p lattice-hermes-adapter --all-targets --locked`: 20 passed,
  3 ignored, 0 failed.
- `cargo clippy -p lattice-hermes-adapter --all-targets --locked -- -D warnings`:
  passed.
- `cargo fmt --all -- --check` and `git diff --check`: passed.
- The ignored real canary
  `tests::reflection_api::wsl_bwrap_socketpair_inherited_fd_canary_is_live_verified`
  ran and failed with the fixed identity-rejection code above.

## Scope and next action

No PostgreSQL, TASK-051, TASK-078 exporter, TASK-041, Issue 7/8, default
branch, merge, deployment, or release action occurred. This record does not
authorize a hash change or a relaxed containment policy. Re-establish an
approved pinned bubblewrap identity, then rerun the exact canary before
changing this terminal status.
