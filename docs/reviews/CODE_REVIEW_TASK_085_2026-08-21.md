# TASK-085 code and security review — 2026-08-21

## Review target

`feature/task-085-parallel-task-governance` against `2027a8c`, limited to the
project-governance checker, its fixtures, TASK-085/SPEC-006, and required
branch-guide metadata.

## Result

No findings (P0=0, P1=0, P2=0, P3=0). Reviewer independence is not proven:
this is a separate read-only pass by the implementing agent because no
independent reviewer was authorized for this worker task.

## Checked properties

- The parallel identity is derived generically from the current canonical
  `feature/task-nnn-*` branch and the unique ticket map; TASK-081/082/083 are
  fixture coverage, not implementation allowlists.
- The existing one-CURRENT-PLANS gate remains intact. The current ticket still
  validates its module, branch guide, and delivery metadata; a different
  parallel ticket must additionally match its branch, module, terminal status,
  branch guide, and delivery policy.
- Ticket-local `display_name_zh_tw` / `display_purpose_zh_tw` is now the
  exactly-once, non-empty Traditional-Chinese presentation authority. The
  shared branch guide remains a legacy fallback only, removing shared JSON
  writes from parallel-safe ticket delivery.
- Unknown, missing, duplicate, branch-mismatched, non-terminal/cancelled,
  unauthorized, and default-branch cases emit errors rather than selecting a
  fallback authority.
- Worktree containment rejects a reparse-resolved root and nesting below an
  ancestor worktree `.git` pointer. A normal ancestor repository `.git`
  directory is not confused with the nested-worktree failure shape.
- The change is Node standard library only; it adds no Git mutation, remote,
  exporter/parser, runtime, database, MCP, Hermes, or credential behavior.

## Evidence

- RED: the added parallel-branch assertions failed against the original
  CURRENT-branch coupling.
- GREEN: `node --test test/project-governance-check.test.js` passed 30/30,
  including generic TASK-081/082/083-shaped ticket-local success and closed
  missing/duplicate/blank/non-Chinese metadata denial cases.
- Governance: `npm.cmd run check` exited 0 with one CURRENT marker.
- Final full regression after the ticket-local repair: `npm.cmd run verify`
  exited 0 with 132/132 tests passing.

## Architecture note

This is a cross-cutting fail-closed validation adjustment, not a new module or
public runtime contract. The checker deliberately does not infer an approved
workspace-root registry; that ownership remains with a future worktree manager
ticket. No ADR or constitution amendment is required.
