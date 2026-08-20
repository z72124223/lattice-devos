---
ticket_id: TASK-085
title: Fail-closed parallel TASK governance
spec_id: SPEC-006
spec_version: 1
module_id: engineering-delivery-finisher
constitution_version: 1.5
status: complete
parallel_safe: true
depends_on: [TASK-078]
branch: feature/task-085-parallel-task-governance
delivery_remote: origin
delivery_repository: github.com/z72124223/lattice-devos
delivery_push: authorized_non_force_feature_branch
delivery_archive: keep_open
allowed_paths:
  - docs/specs/SPEC-006-parallel-task-governance.md
  - docs/tickets/TASK-085-parallel-task-governance.md
  - docs/reviews/CODE_REVIEW_TASK_085_2026-08-21.md
  - scripts/check-project.mjs
  - test/project-governance-check.test.js
  - tools/engineering-status-dashboard/branch-guide.zh-TW.json
---

# TASK-085 — Fail-closed parallel TASK governance

## Objective

Align the project governance check with SPEC-005 v5: a terminal, authorized
parallel TASK branch proves its own identity without modifying the sole
`PLANS.md` CURRENT marker.

## Acceptance conditions

- The check admits only the exact terminal `feature/task-nnn-*` ticket identity
  with safe delivery metadata on a non-CURRENT parallel branch.
- Missing, duplicate, mismatched, non-terminal, unauthorized, cancelled, and
  default-branch cases remain denied; TASK-081, TASK-082, and TASK-083 remain
  generic parallel-identity cases, not hardcoded exceptions.
- A nested/reparse worktree fails before governance success. Approved workspace
  root registration and worktree-manager creation policy are explicitly out of
  scope and remain a follow-up dependency.
- TASK-082, TASK-050, TASK-075, PLANS, HANDOFF, dashboard exporter/parser, and
  all runtime/DB/MCP/Hermes paths remain untouched.

## Verification

```powershell
node --test test/project-governance-check.test.js
npm.cmd run check
npm.cmd run verify
git diff --check
```

## Delivery boundary

This ticket authorizes a non-force feature-branch push only. It does not
authorize a PR, merge, deployment, release, default-branch operation, or native
Codex task archival; `delivery_archive: keep_open` preserves the worker task
for foreman verification.
