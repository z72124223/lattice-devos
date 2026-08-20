---
module_id: engineering-delivery-finisher
name: Engineering Delivery Finisher
constitution_version: 1.4
status: active
---

# Engineering Delivery Finisher module constitution

## Mission

Turn an already committed Codex task checkpoint into one observable,
fail-closed delivery sequence: honor the ticket's explicit feature-push policy,
verify the remote result, refresh the local engineering map, and emit permission
for Codex to archive its current task only after every required gate succeeds.

## Non-Goals

- Create or amend code, commits, tickets, handoffs, tests, or authorization.
- Push or merge a default branch, force-push, create a PR, deploy, release,
  publish, delete work, or manage credentials.
- Own task truth, Git truth, dashboard truth, or Codex App task storage.
- Archive a Codex task directly from repository code.

## Owned Data

- The in-process ordering and result of the bounded finish operation.

Git, the matching TASK ticket, and the Codex App remain the authorities for
repository state, recorded authorization, and task archival respectively.

## Public Contracts

- `node scripts/finish-lattice-delivery.mjs [options]`
- `npm.cmd run delivery:finish`
- Success marker `LATTICE_DELIVERY_FINISHED=1`.
- Conditional archive marker `LATTICE_DELIVERY_READY_TO_ARCHIVE=1`.

## Invariants

1. The current worktree is clean, named, non-default, and matches exactly one
   TASK ticket before any push.
2. Missing, duplicate, malformed, or unknown policy evidence fails closed.
3. Only `authorized_non_force_feature_branch` may push, and only the current
   branch to the ticket's named remote and declared canonical repository
   identity without force.
4. `local_only` skips every push command and still refreshes the map.
5. Authorized delivery is successful only when live remote head and the named
   upstream both equal local `HEAD` after push.
6. Refresh failure prevents overall success and archive readiness.
7. A Git failure after preflight triggers a best-effort map refresh but never
   becomes success.
8. Terminal diagnostics omit secrets, remote URLs, full local paths, command
   bodies, untrusted newlines, and reserved success markers.
9. Repository code emits archive permission but never invokes or impersonates
   the Codex App archive action.
10. Default-branch merge, deployment, release, and other protected authority
    cannot be inferred from a feature-push token.
11. Dashboard output resolves outside the source repository.
12. After refresh, the named branch, clean tree, local head, and any pushed
    remote head are rechecked before success or archive permission.
13. The branch is exactly `feature/task-nnn-*` or `feature/issue-nnn-*`, with
    a lowercase hyphenated slug. A task branch number equals its one unique
    committed terminal TASK ticket; each declared dependency resolves uniquely
    and successfully terminal in that same tree. An issue branch number equals
    its one unique committed terminal ISSUE evidence `issue_id`, whose branch
    is exact; both evidence types configure their remote and delivery policies.
    PLANS is a planning index, never a parallel-delivery lock.
14. Archive permission additionally requires a successful task terminal state;
    failed, blocked, partial, paused, and dependency-waiting tasks stay open even
    when their preservation push and dashboard refresh succeed.
15. The selected remote has exactly one fetch URL and one push URL, both resolve
    to the same credential-free ticket identity, and its config plus live
    default branch are unchanged at the pre-push and final gates.
16. TASK ticket and ISSUE evidence authorization comes from the captured commit
    tree, so Git index visibility flags cannot substitute uncommitted policy
    text; PLANS is not delivery authority.
17. Dashboard generation writes only to a unique external staging directory;
    only a disjoint app-owned directory with the fixed dashboard file set may
    be replaced before archive permission. Unowned entries are never deleted.
18. Repository Git hooks are disabled throughout delivery and cannot extend the
    bounded push or local metadata effects.
19. Refresh success requires both fixed regular dashboard files in staging;
    empty or partial output preserves the prior map and blocks archival.

## Allowed Dependencies

- Node.js standard library.
- Installed Git executable through argument-array process execution.
- Repository TASK tickets, committed ISSUE evidence, and Git metadata.
- The engineering-status dashboard's public exporter command.
- A Codex App archive action performed by Codex after the command succeeds.

No third-party JavaScript package, database, MCP server, daemon, scheduler, or
Git hook is required.

## Forbidden Dependencies

- Shell-evaluated Git arguments, stored credentials, GitHub write APIs, force
  flags, default-branch mutation, merge/deploy/release tools, or destructive Git.
- Repository Git hooks or custom client-side push side effects.
- Direct access to Codex App internal storage or fabricated archive success.
- PostgreSQL or LATTICE task-state mutation.

## Failure, Compatibility, And Migration

- The command exits nonzero and emits no archive-ready marker after any failed
  required step. A best-effort refresh and sanitized failure code preserve diagnosis.
- Existing TASK tickets remain compatible but cannot use the finisher until
  they add explicit policies. Legacy issue branches require their own committed
  ISSUE evidence; GitHub metadata, branch names, and TASK-number collisions do
  not substitute for it.

## Acceptance Gates

- Repository validator: `npm.cmd run check`.
- Focused tests: `node --test test/engineering-delivery-finisher.test.js`.
- Full regression: `npm.cmd run verify`.
- Real non-force feature push, live remote equality, and post-push dashboard
  refresh on the completed TASK-078 checkpoint.
- Code/security, architecture, and integration review with no unresolved P0/P1.

## Change Policy

Changes to push policy, authorization fields, remote selection, default-branch
handling, archive readiness, terminal output, or failure ordering require a new
specification and constitution version. Any default-branch, merge, deployment,
release, credential, or destructive capability requires separate explicit human
approval and must not be added as a minor amendment.

## Amendment History

- 1.0 (2026-08-20): establish the fail-closed feature-delivery and conditional
  Codex task archival boundary for SPEC-005.
- 1.1 (2026-08-20): remove the redundant receipt write after an independent
  reparse-point race review; bind finish to the exact current terminal TASK and
  make failure output marker-safe.
- 1.2 (2026-08-21): bind committed authority and repository endpoint identity,
  reject credential-bearing endpoints, stage external dashboard publication,
  recheck live default/config state, and require an exact named upstream for
  map visibility.
- 1.3 (2026-08-21): admit only uniquely anchored terminal ISSUE evidence for
  legacy issue branches without weakening TASK binding or merging namespaces.
- 1.4 (2026-08-21): make captured ticket/evidence identity, not PLANS current
  focus, the delivery authority; validate declared TASK dependencies locally.
