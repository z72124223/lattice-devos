# Branch and Worktree Plan

## Repository Creation

- Initialize the new repository with primary branch `main`.
- Commit the evidence, plan, specification, ADRs, constitutions, tickets, and
  workflow ledger as the governance baseline.
- Create and check out `feature/phase1-controlled-swarm`.
- Implement TASK-001 through TASK-007 sequentially on that feature branch.

## Why One Feature Branch

The tickets share a new public core contract and are strictly dependency
ordered. Parallel branches would repeatedly overlap `src/index.js`, root
verification scripts, and the evolving orchestration contract. All tickets are
therefore `parallel_safe: false`.

## Worktree Use

- Building LATTICE itself uses the checked-out feature branch.
- The product's Workspace/Git adapter must create isolated worktrees only in
  disposable temporary repositories during Phase 1 tests.
- No test may discover or operate on the playmate repository or another
  user-owned repository.

## Integration Gate

- After local verification and independent reviews, inspect synchronization and
  conflicts against `main`.
- Do not merge `feature/phase1-controlled-swarm` into `main` without explicit
  user authorization.
- Do not push or publish in Phase 1.

