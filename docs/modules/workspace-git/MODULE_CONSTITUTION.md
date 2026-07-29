---
module_id: workspace-git
name: Workspace and Git
version: 1.0
status: active
owner: LATTICE maintainers
last_reviewed: 2026-07-29
---

## Mission

Safely manage the repository/project writer lease, fencing token, task branch,
isolated worktree, and non-conflicting Git metadata operations through an
injectable executor.

## Non-Goals

- Decide who is authorized.
- Let Integrator edit product code.
- Resolve merge conflicts automatically.
- Delete, reset, clean, or modify unrelated user work.
- Provide a distributed lock in Phase 1.

## Owned Data

- Project lock record and monotonically increasing fencing-token counter.
- Worktree identity and task branch metadata created by this module.
- Git command arguments and returned Git evidence.

The repository owns product code. This module never claims ownership of user
files or another repository's Git state.

## Public Contracts

- Inspect repository/base/worktree state.
- Atomically acquire, inspect, validate, and release a writer lease.
- Reject a second lease or stale fencing token.
- Create a sanitized task branch/worktree using argument arrays.
- Return machine-readable changed-path evidence.
- Attempt an approved integration and return conflict evidence without editing.

## Invariants

1. At most one active writer lease exists per repository/project.
2. Only an exact lease ID/fencing token can authorize release or write.
3. No Git command is constructed through a shell string.
4. A worktree stays outside the source checkout and within the configured task
   workspace root.
5. Conflict resolution never chooses ours/theirs or edits product code.
6. Cleanup targets only a verified task-owned disposable worktree.

## Allowed Dependencies

- Node.js filesystem/path/process standard-library APIs.
- Injected command executor and clock/ID sources.
- Task Domain identifiers as plain values.

## Forbidden Dependencies

- Policy decisions, Task Ledger writes, OpenClaw, real model Runtime, user
  credential stores, or broad recursive cleanup.

## Failure, Compatibility, And Migration

Unknown/stale locks fail closed and require explicit reconciliation. Dirty or
mismatched base state blocks worktree creation. Cross-platform paths must be
tested; Windows junctions and POSIX symlinks are rejected unless a future
version provides a proven containment policy.

## Acceptance Gates

| Gate | Evidence | Owner | Required for merge |
|---|---|---|---|
| Lease/fencing tests | `node --test test/workspace-lock.test.js` | Engineering | yes |
| Git command tests | injected executor assertions | Engineering | yes |
| Disposable repository | `node --test test/git-workspace.integration.test.js` | Engineering | yes |
| Conflict fail-closed | integration conflict fixture | Architecture review | yes |

## Change Policy

Lock scope, fencing, cleanup, command construction, conflict, or worktree
containment changes require a versioned amendment, ADR, platform tests, and
responsible-human approval.

## Amendment History

| Version | Date | Decision reference | Summary | Approver |
|---|---|---|---|---|
| 1.0 | 2026-07-29 | ADR-002 | Initial project lock and Git boundary | Current user task |

