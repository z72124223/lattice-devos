# ADR-002: Digest-Bound Approvals, Exclusive Writer, Detection-Only Scope Gate

- Status: accepted for Phase 1
- Date: 2026-07-29
- Decision owner: user request plus fail-closed design

## Context

The request separates execution, merge, and deployment approval and allows only
one product-code writer. A plain text task ID is insufficient to prevent stale
approval reuse, and a post-run Git diff cannot contain a hostile process.

## Decision

### Approval subjects

- Execution approval binds `task_id`, `revision`, and `spec_hash`.
- Merge approval binds `task_id`, `revision`, `reviewed_commit`, and
  `diff_hash`.
- Any change to goal, risk, base commit, scope, acceptance, commands, budget, or
  capability request creates a new revision and invalidates prior approvals.
- Phase 1 uses an injected owner-approval verifier. Live channel identity,
  expiry, nonce persistence, and Telegram/OpenClaw authentication must be proven
  during capability preflight.

### Writer authority

- The repository/project is the Phase 1 lock scope.
- Only `IMPLEMENTER` may obtain a product-code writer lease.
- The lock is acquired atomically and issues a monotonically increasing fencing
  token.
- Planner, mappers, reviewers, and Integrator never receive a code-writer lease.
- Integrator may execute approved, non-conflicting Git integration. A merge
  conflict blocks integration and must become a new Implementer task.
- Stop follows `STOPPING` to `CANCELLED` only after runtime termination and
  lease revocation are evidenced.

### Scope evidence

- All changed paths must be canonical, repository-relative, and allowed.
- Absolute paths, traversal, `.git/**`, symlinks/junctions, and escaped targets
  fail closed in Phase 1.
- Rename checks both old and new paths.
- Scope Check reports violations and never edits or repairs files.
- Passing Scope Check is a detection result, not proof of OS sandboxing or
  hostile-process containment.

## Consequences

- Stale or substituted plans cannot reuse approvals.
- A second writer and a conflict-editing Integrator are rejected.
- Real Runtime acceptance remains blocked until process, filesystem, secret, and
  network isolation are separately proven.

