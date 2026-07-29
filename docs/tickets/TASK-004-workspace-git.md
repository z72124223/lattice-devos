---
ticket_id: TASK-004
spec_id: SPEC-001
module_id: workspace-git
constitution_version: 1.0
status: ready
parallel_safe: false
depends_on:
  - TASK-001
  - TASK-003
allowed_paths:
  - src/workspace/**
  - src/index.js
  - test/workspace-lock.test.js
  - test/git-workspace.integration.test.js
  - PLANS.md
  - docs/tickets/TASK-004-workspace-git.md
  - docs/workflow/WORKFLOW_LEDGER.md
likely_files:
  - src/workspace/project-lock.js
  - src/workspace/git-workspace.js
  - test/workspace-lock.test.js
  - test/git-workspace.integration.test.js
branch: feature/phase1-controlled-swarm
---

## Objective

Deliver the exclusive repository/project writer lease with fencing and the
argument-array Git worktree/integration adapter proven only on disposable
repositories.

## Acceptance Criteria

- [ ] SPEC-001 AC-05.
- [ ] SPEC-001 AC-08.
- [ ] Merge conflict returns evidence and never edits a product file.
- [ ] Cleanup cannot target an unowned or broad path.

## Non-Goals

- Policy decisions, scope classification, automatic conflict resolution, or
  user-repository cleanup.

## Module And Constitution Constraints

Use `workspace-git` v1.0. Unknown locks fail closed. Shell command strings,
force/reset/clean, and ours/theirs conflict selection are forbidden.

## Dependencies And Overlap

Blocked on Task Domain identifiers and Policy writer decisions. Not
parallel-safe because its lease contract is consumed by orchestration.

## TDD Behaviors

1. First exact writer acquires a lease/fencing token.
2. Second writer and wrong/stale token deny.
3. Exact release succeeds; unknown/stale lock is not auto-broken.
4. Git branch/worktree arguments are sanitized arrays.
5. Disposable repository worktree starts at exact base commit.
6. Changed-file evidence includes required Git states.
7. Conflicting integration blocks without resolution.
8. Only verified task-owned disposable worktree cleanup succeeds.

## Verification

| Check | Command or service | Expected evidence |
|---|---|---|
| Lock tests | `node --test test/workspace-lock.test.js` | exit 0 |
| Disposable Git integration | `node --test test/git-workspace.integration.test.js` | exit 0 |
| Full current suite | `npm test` | exit 0 |

## Human Gate

none
