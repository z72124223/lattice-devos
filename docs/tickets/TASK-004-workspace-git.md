---
ticket_id: TASK-004
spec_id: SPEC-001
module_id: workspace-git
constitution_version: 1.0
status: complete
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
  - docs/tickets/TASK-005-scope-check.md
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

- [x] SPEC-001 AC-05.
- [x] SPEC-001 AC-08.
- [x] Merge conflict returns evidence and never edits a product file.
- [x] Cleanup cannot target an unowned or broad path.

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

## TDD Evidence

Every implementation group began with an observed failing focused test:

| RED behavior | Observed failure before implementation |
|---|---|
| Project lock and Git adapter | missing modules / missing `verifyIntegration` |
| Staged/unstaged evidence | `states` was absent |
| Ownership and containment | forged marker removed another worktree; junction inspection escaped |
| Fail-before-side-effect roots | external lock/ownership directories were created before rejection |
| Lock integrity | malformed stored record, invalid clock, and missing counter were accepted |
| Concurrent first acquire | loser was misclassified `LOCK_UNKNOWN_STATE` |
| Git/lock interoperability | exact lock metadata caused `REPOSITORY_DIRTY` |
| Git execution safety | `post-checkout` ran; repo-local external driver was accepted |
| Integration recovery | injected evidence failure left a conflicted worktree and marker |
| Root input safety | empty/NUL root reached path resolution |

GREEN evidence on 2026-07-29:

- `node --test test/workspace-lock.test.js`: 9 passed.
- `node --test test/git-workspace.integration.test.js`: 11 passed.
- 20 repeated concurrent-acquire trials: 20 passed.
- `npm.cmd run verify`: exit 0, `check=ok files=55 constitutions=7`,
  38 tests passed.
- `git diff --check`: exit 0.

The adapter disables repository hooks, disables `core.fsmonitor`, rejects
repo-local external filter/merge/diff drivers, validates canonical
ownership/common-directory evidence, and never uses shell interpolation,
`force`, `reset`, `clean`, or ours/theirs conflict selection.

## Human Gate

none
