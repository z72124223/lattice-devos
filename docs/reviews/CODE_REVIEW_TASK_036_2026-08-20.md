# TASK-036 Code Review

## Review Target

- Branch: `feature/task-036-codex-app-server-repair`
- Reviewed HEAD: `dfa33dedb94c9a19bdfcce8306e61e2c6a0681f4`
- Scope: the authoritative TASK-036 status record, TASK-032 branch
  disambiguation, current-plan marker, and the Codex app-server repair evidence
  introduced through `9c13e5f`.
- Reviewer independence: not proven; this is a separate read-only review pass
  in the implementation worktree.

## Finding

### P1 - Official delivery acceptance is still not reproducible

`TASK-032` requires a repeatable PowerShell command proving the full delivery
path, including restart/status replay, before it can close. The one authorized
official run changed the isolated repository and persisted a completed receipt,
but its exact PostgreSQL data directory was removed before the separate restart
status invocation. The consumed official one-shot cannot be rerun under this
checkpoint, and an unrelated cluster cannot substitute for the missing receipt.

Evidence:

- `docs/tickets/TASK-032-executable-codex-postgres-delivery.md:97` leaves the
  repeatable full-path acceptance unchecked.
- `docs/tickets/TASK-032-executable-codex-postgres-delivery.md:156` records the
  missing same-receipt restart replay.
- `HANDOFF.md:53` through `HANDOFF.md:55` confirm the exact data directory no
  longer exists and forbid substitution.
- `PLANS.md:85` and `PLANS.md:744` retain TASK-032 as `NEEDS_REVIEW` and
  `in-progress`.

Resolution: no safe local code change can recreate this evidence. Keep TASK-036
as `partial`, keep `delivery_archive: keep_open`, and require a newly authorized
official attempt with a fresh latch and durable restart/status replay before
any terminal `complete` claim.

## Verification

- `npm.cmd run check`: PASS (`files=330`, `constitutions=22`, `tickets=25`,
  `current_tasks=1`).
- `cargo test -p lattice-contracts -p lattice-ports -p lattice-orchestrator
  -p lattice-codex-adapter -p lattice-runtime`: PASS.
- `powershell -NoProfile -ExecutionPolicy Bypass -File
  scripts/run-lattice-delivery.ps1 -TestRuntimeTerminalEnvelope`: PASS.
- `powershell -NoProfile -ExecutionPolicy Bypass -File
  scripts/run-lattice-delivery.ps1 -ScriptedDeadlineRegression`: PASS.
- `git diff --check`: PASS.

## Review Result

No new P0/P2/P3 implementation defect was found in the reviewed repair paths.
The P1 acceptance-evidence gap blocks completion of the underlying Codex
app-server delivery ticket. TASK-036 correctly projects this as `PARTIAL`, not
`COMPLETE`.
