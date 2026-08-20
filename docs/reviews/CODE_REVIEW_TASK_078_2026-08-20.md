# TASK-078 independent code and security review — 2026-08-21

## Verdict

`PASS`. Independent reviewer: Codex subagent `task078_code_review` (Hilbert).
Unresolved findings: P0=0, P1=0, P2 runtime/security=0.

## Findings resolved during review

| Priority | Finding | Resolution and regression |
| --- | --- | --- |
| P1 | Cached `origin/HEAD` could hide a changed live default branch. | Query live symbolic HEAD before push and at final gate; stale-cache and concurrent-default tests. |
| P1 | Branch/head/config could change between authority checks and push. | Capture branch, exact SHA, clean state, remote URLs and endpoint identity; recheck before push/final; push the captured SHA to the captured endpoint. |
| P1 | Exact-SHA push did not establish an upstream, so the map showed `no-upstream`. | Set and verify the named upstream at the exact SHA. |
| P1 | Named remote could use different fetch/push endpoints or an unauthorized repository. | Require one fetch and one push URL, identical canonical endpoint, and exact ticket `delivery_repository`. |
| P1 | A failed or stale TASK ticket could gain push/archive authority. | Require exact `feature/task-nnn-*`, unique committed ticket, unique committed PLANS current marker, terminal state, and successful state for archive. |
| P1 | `skip-worktree` could hide substituted policy text. | Read tickets and PLANS from the captured commit tree; regression proves hidden working text cannot grant push. |
| P1 | Failure text could inject the archive marker; missing CLI values could fall back to cwd. | Fixed single-line diagnostics and strict option values; subprocess regressions. |
| P1 | Output junction races could write into the source repository. | Generate in a unique external sibling staging directory and recheck containment before publication, including failed-preflight refresh. |
| P1 | Replacing an arbitrary output directory could delete unrelated data. | Require disjoint app-owned output, fixed files/marker, bounded cleanup, and preserve/reject sentinel or repository-ancestor targets. |
| P1 | Refresh exit 0 with no files could falsely report `dashboard=REFRESHED`. | Require both regular `index.html` and `status.json`; zero-output preserves the old map and fails. |
| P1 | Failed task status could still emit archive permission. | Separate terminal from successful-terminal states; failed/blocked/partial work never archives. |
| P2 | Raw unexpected errors could leak local paths or endpoint credentials. | Fixed CLI diagnostics and reject URL userinfo/query/fragment before live network Git. |
| P2 | Repository Git hooks could add unbounded side effects. | Disable hooks for all finisher Git commands and use push `--no-verify`; side-effect regression. |

The originally proposed delivery receipt was removed under SPEC-005 v2 review
because writing it after the final containment gate created a reparse-point
race. SPEC-005 v3 instead uses GitHub, the owned map projection, and the exact
archive marker as the bounded observable result.

## Final reviewer evidence

- `node --test test/engineering-delivery-finisher.test.js`: 35/35 PASS.
- `node --test test/project-governance-check.test.js`: 21/21 PASS.
- `npm.cmd run check`: PASS (`files=520 constitutions=27 tickets=49 current_tasks=1`).
- `git diff --check`: PASS.

## Residual risk

Git hosting does not provide an atomic lock across symbolic default-branch
lookup and push; the command therefore fails if drift remains observable at the
final gate. Windows rename behavior while the local page is open is covered by
the final live TASK-078 delivery rather than claimed from fixture tests.
