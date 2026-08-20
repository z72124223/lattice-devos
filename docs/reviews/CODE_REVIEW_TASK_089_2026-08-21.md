# TASK-089 Code and Security Review — 2026-08-21

## Target

`feature/task-089-evidence-subject-governance` relative to
`e34bc9bfcf18c71e771f704d50128e1fbeba53ea`.

## Review result

Independent read-only review: **No findings** after remediation.

- `evidence_subjects` is parsed separately from `depends_on` in both the
  finisher and exporter; only dependencies require successful terminal state.
- Every declared subject resolves from the captured `HEAD` tree to one legal
  filename/`ticket_id` pair. Missing, duplicate, malformed, self-referential,
  overlapping, and provenance-cycle cases fail closed.
- A nonterminal evidence subject is exposed only as provenance and does not
  promote the cited TASK or block delivery of the citing TASK.
- TASK-089 cites TASK-050 and TASK-075, the actual reconciled subjects that
  exist uniquely in its captured tree; it does not alter TASK-082 or TASK-083.
- Protocol version 1.2 and the project validator include a negative regression
  for the new governance language.

## Evidence

- RED then GREEN focused tests cover the new finisher, exporter, and project
  governance behavior.
- `node --test test/project-governance-check.test.js`: 33/33 PASS.
- `npm.cmd run check`: PASS.
- 修復後完整 `npm.cmd run verify`: 139/139 PASS.
- `git diff --check`: PASS.

## Residual risk

The dashboard is a read-only projection, not task truth. No unresolved review
finding remains; delivery still requires the finisher's post-push live checks.
