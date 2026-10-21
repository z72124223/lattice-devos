# V2 Branch and Worktree Preservation Plan

## Current State

- Repository branch: `feature/phase1-controlled-swarm`
- Observed HEAD before V2 planning: `06c3954`
- The worktree was already dirty with eight TASK-004 security-review paths.
- V2 planning now intentionally adds/changes governance documents in the same
  worktree.
- No reset, clean, branch switch, stash, commit, merge, push, or worktree move
  is authorized by the planning gate.

## Immediate Rule

Do not start Rust implementation in the dirty source worktree and do not
continue old TASK-005 through TASK-007. The user approved the V2 direction on
2026-07-29. Rust bootstrap work is restricted to this dedicated sibling
worktree on `feature/v2-rust-postgres-bootstrap`.

## Required Preservation Gate

Before creating a V2 implementation worktree:

1. Re-read Git status and the complete diff.
2. Separate:
   - V1 TASK-004 code/test/document WIP;
   - V2 replan documents;
   - any unrelated user changes.
3. Resolve the known V1 fencing safe-integer finding or explicitly classify it
   as preserved incomplete characterization evidence.
4. Produce a reversible preservation artifact or intentional local commit for
   each owned change group.
5. Verify no untracked or ignored relevant path is omitted.
6. Obtain user approval for any commit/branch disposition that will become the
   implementation baseline.
7. Record the exact DAG: observed V1 base/head, any V1 WIP preservation commit,
   the V2 governance commit, and the exact commit from which the V2 branch is
   created.
8. Create a manually reviewed retained-fixture manifest. It must exclude the
   known fencing overflow bug and any unverified TASK-004 behavior from
   compatibility requirements.

Do not use reset, clean, checkout-overwrite, force, or broad recursive moves.

## Active V2 Worktree Shape

```text
source checkout:
  feature/phase1-controlled-swarm
  preserved V1 prototype and replan evidence

separate sibling worktree:
  feature/v2-rust-postgres-bootstrap
  approved SPEC-002/ADRs/constitutions/tickets
```

The V2 worktree was created from `06c3954`. Approved governance files were
copied as uncommitted, inspectable changes. No preserved V1 `src/` or `test/`
change was copied, committed, reset, cleaned, merged, or pushed.

## Ticket And Parallelism Policy

- The initial Rust contracts, canonical serialization, PostgreSQL interfaces,
  and shared IPC schemas are dependency-forming work and are not
  `parallel_safe`.
- Provider adapters may become parallel-safe only after their common traits,
  evidence envelope, fake fixtures, and schema/version policy are accepted.
- Every parallel ticket uses its own branch/worktree and disjoint
  `allowed_paths`.
- No two tickets may share a PostgreSQL migration, public contract, lockfile,
  generated schema, or product worktree.

## Verification Before Integration

- Verify the target/base commit and branch synchronization.
- Verify complete changed-path and ignored/untracked evidence.
- Run focused and full checks in the integrated V2 result.
- Run independent code and architecture review.
- Verify migration compatibility and disposable database tests.
- Verify no V1/V2 dual writer or dual event truth exists.
- Treat remote CI and branch protection as unverified until observed.
- Never merge the primary branch without explicit user authorization.
