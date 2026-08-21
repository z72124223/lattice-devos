---
module_id: engineering-delivery-finisher
name: Engineering Delivery Finisher
constitution_version: 2.0
status: active
---

# Engineering Delivery Finisher module constitution

## Mission

Provide an opt-in, fail-closed boundary for an explicitly authorized remote
feature delivery: validate its committed terminal policy, perform only the
authorized non-force push, verify remote equality, optionally refresh the local
engineering map, and emit archive readiness.

## Non-Goals

- Define ordinary development, local completion, checkpoints, testing, review,
  task creation, planning, or dashboard presentation.
- Repair or require tickets, handoffs, branch labels, plans, or historical
  branches before ordinary product verification.
- Push or merge a default branch, force-push, deploy, release, publish, manage
  credentials, delete work, or archive Codex tasks directly.

## Owned Data

Only the in-process ordering and result of one explicitly invoked finish
operation. Git, tests, task state, runtime receipts, and Codex task storage keep
their existing owners.

## Public Contracts

- `node scripts/finish-lattice-delivery.mjs [options]`
- `npm.cmd run delivery:finish`
- `LATTICE_DELIVERY_FINISHED=1`
- `LATTICE_DELIVERY_READY_TO_ARCHIVE=1`

## Invariants

1. Invocation is optional and never inferred from ordinary completion.
2. The current worktree is clean and non-default before any push.
3. Only an explicit committed non-force feature policy may push.
4. The selected remote and named upstream must equal local `HEAD` after push.
5. Missing, duplicate, malformed, credential-bearing, or unknown policy fails
   closed and emits no archive-ready marker.
6. The command never merges, deploys, releases, force-pushes, deletes work, or
   mutates Codex task storage.
7. Diagnostics remain bounded and secret-free.
8. Dashboard failure may fail this optional delivery command but cannot change
   product test results or automatically create repair work.

## Allowed Dependencies

Node.js standard library, Git through argument-array execution, a committed
terminal delivery record, and the read-only dashboard exporter.

## Forbidden Dependencies

Shell-evaluated Git, stored credentials, force flags, default-branch mutation,
merge/deploy/release tools, PostgreSQL mutation, Git hooks, or direct Codex App
storage access.

## Failure, Compatibility, And Migration

The existing command and strict terminal metadata remain compatible for callers
that explicitly choose remote delivery. Callers that only need local completion
stop invoking the finisher; no ticket migration is required.

## Acceptance Gates

- Focused finisher tests for any finisher code change.
- `npm.cmd run check` for repository structure.
- `npm.cmd run verify` only when finisher or shared Node behavior changes.
- A real remote equality check only when a remote delivery is explicitly
  authorized and actually performed.

## Change Policy

Default-branch, merge, deployment, release, credential, force, destructive, or
public-network capability requires separate explicit human authority. Ordinary
workflow policy must not be added back to this module.

## Amendment History

- 1.x: mandatory per-task delivery and dashboard/archive coordination.
- 2.0 (2026-08-21): make the finisher opt-in, preserve its remote safety
  boundary, and remove it as an ordinary-development prerequisite.
