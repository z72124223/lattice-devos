---
ticket_id: TASK-004
spec_id: SPEC-001
module_id: workspace-git
constitution_version: 1.0
status: partial
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
| Endpoint-diff opacity | a staged change disappeared when the worktree copy returned to base |
| Ownership and containment | forged marker removed another worktree; junction inspection escaped |
| Fail-before-side-effect roots | external lock/ownership directories were created before rejection |
| Lock integrity | malformed record, invalid clock, missing counter, and rolled-back lease were accepted |
| Concurrent first acquire | loser was misclassified `LOCK_UNKNOWN_STATE` |
| Git/lock interoperability | exact lock metadata caused `REPOSITORY_DIRTY` |
| Git execution safety | hooks and local/global/env/include/worktree filter configurations executed external marker programs |
| Ignored write evidence | ignored files were absent from changed-path evidence and could be removed silently |
| Creation/integration recovery | injected post-create/evidence failures left worktrees, markers, or branches |
| Branch provenance | failed creation could delete a pre-existing task branch |
| Root input safety | empty/NUL root reached path resolution |

GREEN evidence on 2026-07-29:

- `node --test test/workspace-lock.test.js`: 10 passed.
- `node --test test/git-workspace.integration.test.js`: 25 passed.
- 20 repeated concurrent-acquire trials: 20 passed.
- `npm.cmd run verify`: exit 0, `check=ok files=55 constitutions=7`,
  53 tests passed.
- `git diff --check`: exit 0.

These results belong to an earlier TASK-004 snapshot. Later uncommitted review
changes have not received a complete current verification result: the V2
replanning pass timed out while running `npm.cmd run verify`, and fencing-token
increment still needs a safe-integer/overflow fail-closed regression. This
ticket is therefore `partial`, not current-tree complete.

The adapter disables repository hooks and `core.fsmonitor`; isolates
system/global Git config and attributes; removes inherited Git configuration
injection; rejects local/worktree includes and external filter/merge/diff
drivers; rechecks every driver-sensitive command; and reports ignored files as
scope evidence. It validates canonical ownership/common-directory evidence and
never uses shell interpolation, `force`, `reset`, `clean`, or ours/theirs
conflict selection.

## Human Gate

none

## 2026-08-25 reconciliation

This ticket remains `partial`. The current implementation validates a retained
fencing counter before use, but the next token is still computed as
`current + 1` without a fail-closed check that the result remains a safe
integer. No current regression covers `Number.MAX_SAFE_INTEGER`.

Next action: add a failing overflow test and reject before writing the counter
or creating a lease whenever the next fencing token is not a safe integer.
