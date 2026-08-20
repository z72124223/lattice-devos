# TASK-077 V2 independent code and security review — 2026-08-20

## Target and independence

- Branch: `feature/task-077-engineering-status-dashboard`.
- Reviewed base: `8351d0205e27afd9c3d2d07cce1580a61b4e3a77`.
- Scope: the complete V2 uncommitted diff for SPEC-004 v2, module constitution
  1.1, TASK-077, exporter/schema, Chinese guide, static tree UI, governance
  validator, and focused tests.
- Independence: read-only review and three bounded re-reviews were performed by
  the separate `task077_code_review` agent. The reviewer did not modify files.

## Findings and resolutions

| Priority | Finding | Resolution evidence |
| --- | --- | --- |
| P1 | A page older than 24 hours could retain its generated green eligibility. | UI eligibility now checks the timestamp, rejects excessive future skew, reloads at expiry, and disables selection/copy on an old or untrusted timestamp. |
| P1 | Git ancestry failure produced a partial-looking tree without affecting completeness or eligibility. | `sources.gitAncestry` and completeness now record failure; every recommendation and selection fails closed. A corrupt-ref fixture proves the path. |
| P2 | A detached worktree could become same-commit anchor ahead of a named branch. | Anchor priority is default, then named stable, then detached/prunable; same-head descendant regression passes. |
| P2 | Protocol prose did not prove the current Git branch had a Chinese guide entry in ticket scope. | Project check compares the live Git branch, parses only `allowed_paths`, requires the guide path and Han text, and rejects mismatch, decoy-list, missing, and English-only fixtures. |
| P2 | Clipboard fallback could report success after a false copy result. | Both modern and fallback copy paths report success only on evidence; false/throw gives a manual-copy message and temporary DOM cleanup occurs in `finally`. |
| P2 | Schema 2.0 could replace a good page without required tree/dispatch structure. | Writer validation now proves roots, unique keys/edges, reverse edges, inbound counts, acyclicity, full reachability, Chinese display, eligibility, freshness, and recommendation validity before atomic replacement. |
| P3 | Future-dated snapshots were described only as expired. | Badge, header, node reason, and footer now say the page is expired or its time is untrusted. |
| P3 | Clipboard exception could leave a hidden temporary textarea. | Fallback cleanup now runs in `finally` for success, false, and throw paths. |

## Verification observed by review

- Focused dashboard suite after all material repairs: 19/19 passed.
- Focused governance suite: 17/17 passed.
- Full repository verification before the final two narrow P3 text/cleanup
  fixes: 74/74 passed.
- Final narrow affected tests: 3/3 passed; template JavaScript syntax check and
  `git diff --check` passed.
- Live refresh: schema 2.0, 40 nodes, one root, Git ancestry available, zero
  unmapped current nodes, and fail-closed TASK-051/TASK-077 states.

## Final decision

- Findings: P0=0, P1=0, P2=0, P3=0 (`No findings` after remediation).
- Residual gap: clipboard and expiry timer have logic/string regressions but no
  automated real-browser interaction test. Browser launch was exercised; no
  pixel-level visual acceptance is claimed.
- Blocker status: `PASS`, blocker-free and ready for architecture review.
