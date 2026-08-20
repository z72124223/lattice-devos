# TASK-084 Workflow Ledger

## Identity And Base

- Branch: `feature/task-084-workflow-learning-loop`
- Worktree: `lattice-worktrees/task-084-workflow-learning-loop`
- Base: `47bf0a5f2d8cd4bc8c41ad499eba372ac0b17e40`
- Dependency: TASK-079 for durable integration only
- Delivery: local commit; no push, merge, deployment, release, or archive

## TDD Evidence

1. Initial RED: focused Cargo test exited 1 with unresolved workflow-learning
   public contracts.
2. Suspicion/topology RED: focused Cargo test exited 1 with missing queue,
   TASK-083 topology, stop-claim, action, and policy-rule types.
3. GREEN: focused suite passed 15 tests after the minimal pure contracts.
4. Security-review RED: focused suite exited 1 with missing policy rationale/
   scope types; follow-up tests also exposed untrusted support/path handling.
5. GREEN: focused suite passed 17 tests after fail-closed construction and
   cross-case path promotion.
6. Review RED: active-window counterexample was absent. A separate attempted
   two-filter Cargo command exited 1 before test execution because Cargo
   accepts one TESTNAME; it was classified as setup failure and not repeated.
7. Final focused GREEN: workflow-learning integration suite passed 18 tests.

## Verified Checks

- Focused TASK-084 suite: 18 passed, 0 failed.
- Strict Clippy for standalone crate/all targets: exit 0.
- Cargo fmt check: exit 0.
- Module constitution validator: valid, zero errors/warnings.
- TASK-048 worker observation boundary: 9 passed.
- TASK-049/window closure boundary: 3 passed.

## Known Repository Check

`npm.cmd run check` exits 1 on the clean base because shared `PLANS.md` has two
CURRENT TASK markers. TASK-084 deliberately does not edit that active TASK-079
overlap. This is a repository/shared-current gate, not a focused crate failure.

An optional root `cargo test --workspace --locked` build was started but its
detached command exit was not captured; it is therefore **NOT VERIFIED** and is
not counted as acceptance. A guarded stop attempt made no change because the
exact TASK-084 Cargo process had already exited. The concurrent TASK-088 Cargo
process was identified by its target path and left untouched.

## Safety Notes

No reset, clean, manual worktree move, deletion, live PostgreSQL, TASK-051,
network publication, force operation, or protected authority action occurred.
