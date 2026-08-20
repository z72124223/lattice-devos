# TASK-077 independent code and security review — 2026-08-20

## Target

- Branch: `feature/task-077-engineering-status-dashboard`
- Base: `6e393f687a02791c96030123e16fc18e1a723932`
- Scope: exact uncommitted TASK-077 worktree diff against SPEC-004 v1, the
  engineering-status-dashboard constitution v1.0, and TASK-077.
- Independence: first pass performed read-only in a separate agent context.

## First-pass findings and resolutions

| Priority | Finding | Resolution evidence |
| --- | --- | --- |
| P1 | The `.cmd` repository argument could merge the trailing separator with the closing quote. | Launcher now passes `%~dp0.` and a Windows-only integration test executes the real `.cmd` from a path with spaces. |
| P1 | Windows `--open` used an empty PowerShell `$args` value and reported success before proving launch. | The exporter now passes the exact file as an `explorer.exe` argv entry, awaits the spawn event, and reports opened only afterward; injected-process regression proves the exact argv. |
| P2 | An ISSUE branch without task evidence was inferred as `IN_PROGRESS`. | No ticket outcome now maps to `UNKNOWN`; Git/PR evidence stays separate. |
| P2 | Duplicate or malformed TASK tickets could be selected arbitrarily and marked complete evidence. | Ticket collection now requires exactly one matching file with exact `ticket_id`, `status`, and `branch`; conflict and missing-ticket regressions fail closed. |
| P2 | `synced` compared only a potentially stale local tracking ref. | One bounded read-only `git ls-remote --heads` per configured remote binds synchronization to a live remote head. An external peer-advance regression proves stale tracking becomes `remote-changed`. |
| P2 | Protocol v1.0.2 presence was enforced, but its new refresh rule was not. | The repository validator now requires both `npm.cmd run status:refresh` and the non-authoritative-projection boundary; a removal regression is rejected. |
| P2 | Repository prose could expose an absolute local path in the default page. | Plain-text normalization redacts common quoted, Windows, and Unix absolute local paths before snapshot/rendering; the injection fixture now proves both path families absent. |

## Current verification

- `node --test test/engineering-status-dashboard.test.js test/project-governance-check.test.js`: PASS, 19/19.
- `npm.cmd run verify`: PASS, 57/57 after all review remediations.
- `git diff --check`: PASS.
- Live `npm.cmd run status:open`: exit 0, 39 items, partial evidence retained,
  `LATTICE_STATUS_OPENED=1`.
- Live snapshot: Git remote source available; GitHub source available;
  TASK-051 remains explicit `FAIL`, clean, live-remote `synced`, PR #13 CI passing;
  TASK-077 remains `IN_PROGRESS`, dirty before commit, no upstream.

## Second-pass findings and resolutions

The independent re-review confirmed the two P1 and six of the original P2
findings closed. It retained one privacy P2, found one status-compatibility P2,
and found one malformed-status P3:

| Priority | Finding | Resolution evidence |
| --- | --- | --- |
| P2 | Repository-native `status: in-progress` was not normalized. | A centralized status map now normalizes hyphen/space to underscore and covers every status currently present in repository tickets. Live TASK-038 now reads `IN_PROGRESS`. |
| P2 | Unquoted forward-slash drive paths, UNC paths, `/root`, and `/data` could bypass redaction. | Redaction now covers quoted/backticked absolute paths, both drive separators, UNC, and common Unix roots. The fixture asserts each format independently absent. |
| P3 | A non-empty but unknown status became `UNKNOWN` with evidence marked complete. | Exact ticket validation now rejects unrecognized status as a source error, retaining the card as `UNKNOWN` with partial evidence. |

## Final current verification

- Dashboard + governance focused tests: PASS, 21/21.
- `npm.cmd run verify`: PASS, 59/59.
- `git diff --check`: PASS.
- Live refresh: 39 items; repository-native TASK-038 `in-progress` maps to
  `IN_PROGRESS`; TASK-051 remains explicit `FAIL` with live remote verification.

Final exact-diff independent re-review is pending. Until that pass, blocker
status remains `BLOCKED`.

## Third-pass finding and resolution

The next exact-diff review closed the second-pass findings and found one P2:
duplicate frontmatter keys such as two conflicting `status` values were silently
last-write-wins. Frontmatter parsing now records duplicate scalar keys, and
ticket validation rejects the entire source before outcome classification. The
regression proves conflicting `in_progress` and `complete` values remain
`UNKNOWN` with partial evidence.

- Dashboard + governance focused tests after this repair: PASS, 22/22.
- `npm.cmd run check`: PASS, 510 files, 26 constitutions, 48 tickets, one
  current task.
- `git diff --check`: PASS.

Final independent confirmation found no new issue in the narrow repair or its
adjacent hunks. Targeted duplicate-frontmatter regression passed independently.

## Final decision

- Findings: P0=0, P1=0, P2=0, P3=0.
- Residual gap: path redaction is intentionally heuristic for uncommon mount
  roots; verified current-machine Windows drive, forward-slash drive, UNC,
  `/home`, `/root`, and `/data` formats are covered.
- Blocker status: `PASS`, blocker-free and ready for architecture review.
