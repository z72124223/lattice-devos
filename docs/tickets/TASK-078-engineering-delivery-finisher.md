---
ticket_id: TASK-078
title: Fail-closed engineering delivery finisher
spec_id: SPEC-005
spec_version: 5
module_id: engineering-delivery-finisher
constitution_version: 1.4
status: complete
parallel_safe: false
depends_on: [TASK-077]
branch: feature/task-078-engineering-delivery-finisher
delivery_remote: origin
delivery_repository: github.com/z72124223/lattice-devos
delivery_push: authorized_non_force_feature_branch
delivery_archive: after_success
allowed_paths:
  - AGENTS.md
  - PLANS.md
  - HANDOFF.md
  - package.json
  - docs/contracts/ENGINEERING_PROTOCOL_V1.md
  - docs/issues/**
  - docs/specs/SPEC-005-engineering-delivery-finisher.md
  - docs/modules/engineering-delivery-finisher/MODULE_CONSTITUTION.md
  - docs/tickets/TASK-078-engineering-delivery-finisher.md
  - docs/reviews/WORKFLOW_LEDGER_TASK_078_2026-08-20.md
  - docs/reviews/CODE_REVIEW_TASK_078_2026-08-20.md
  - docs/reviews/ARCHITECTURE_REVIEW_TASK_078_2026-08-20.md
  - docs/reviews/INTEGRATION_TASK_078_2026-08-20.md
  - scripts/check-project.mjs
  - scripts/finish-lattice-delivery.mjs
  - tools/engineering-status-dashboard/branch-guide.zh-TW.json
  - test/engineering-delivery-finisher.test.js
  - test/project-governance-check.test.js
---

# TASK-078 — Fail-closed engineering delivery finisher

## Objective

Implement SPEC-005 as one dependency-free Node command that turns a clean,
committed TASK branch into a verified local-only or authorized non-force
delivery, refreshes the engineering map, and permits
Codex to archive the current task only after full success.

## Acceptance conditions

- Every SPEC-005 acceptance criterion is covered by focused automated evidence.
- The existing dashboard exporter remains read-only and is called through its
  public command rather than gaining Git mutation authority.
- Missing/unknown policies, default or detached branch, dirty tree, branch or
  ticket mismatch, rejected push, remote mismatch, and refresh failure emit no
  archive-ready signal.
- A split, ambiguous, changed, or ticket-mismatched remote endpoint fails
  closed; a successful push establishes an exact named upstream so the map can
  prove GitHub synchronization.
- Output that resolves inside the repository and local/remote changes during
  refresh fail the final state gate and emit no archive-ready signal.
- This branch is finally delivered by the new command itself: non-force push,
  exact remote equality, map refresh, success marker, then the native Codex
  archive action.
- No PR, default-branch merge, deployment, release, public host, credential
  change, worktree deletion, or force operation occurs.

## ISSUE compatibility migration

Legacy `feature/issue-nnn-*` branches do not create, reuse, or impersonate a
`TASK-nnn` ticket. Before a terminal issue branch can run the finisher, its own
committed tree must contain exactly one `docs/issues/ISSUE-nnn-*.md` record:

```yaml
---
issue_id: ISSUE-007
status: complete
branch: feature/issue-007-resource-aware-scheduler
delivery_remote: origin
delivery_repository: github.com/z72124223/lattice-devos
delivery_push: authorized_non_force_feature_branch
delivery_archive: keep_open
---
```

The issue number, exact branch, one committed terminal evidence record, remote
identity, and policies are all required. Missing/GitHub-only evidence,
duplicates, number mismatch, arbitrary feature branches, non-terminal state,
or an existing TASK number collision fail closed. Issue 7/8 windows add their
own evidence and rerun the finisher; this TASK-078 branch does not alter them.

For parallel TASK branches, the committed terminal ticket itself is delivery
authority: its branch number and exact `branch` must match, and every declared
`depends_on` TASK must resolve uniquely to a successfully terminal ticket in
the same captured commit tree. `PLANS.md` is a shared planning index, not a
delivery lock; TASK-042 may finish while it names TASK-033 without either
window editing PLANS.

## TDD evidence plan

1. RED: prove the public finisher module/command and required governance fields
   do not yet exist.
2. GREEN: implement policy parsing, current-task binding, bounded
   push/verification, refresh, final-state gate, and marker-safe output one
   behavior at a time.
3. REFACTOR: keep process execution injectable for tests and keep policy/error
   decisions independent of CLI formatting.

## Verification

```powershell
npm.cmd run check
node --test test/engineering-delivery-finisher.test.js
node --test test/project-governance-check.test.js
npm.cmd run verify
git diff --check
```

## Human gate

The user's two direct replies on 2026-08-20 approve the one-command finisher,
the bounded non-force push of this feature branch, and archival of this current
Codex task only after every delivery gate succeeds. PR, default-branch merge,
deployment, release, publication, credentials, destructive cleanup, and
archival after failure remain unauthorized.

## Completion evidence

- Finisher focused: 35/35 PASS; governance: 21/21 PASS.
- Full feature and exact combined-default verification: 114/114 PASS each.
- Independent code/security review: PASS, unresolved P0/P1/P2 runtime = 0.
- Architecture review and zero-conflict exact default-target integration: PASS.
- Implementation checkpoint: `f04b462571e6bdd052db9c4cd343bfc26d158628`.

The final clean handoff commit is delivered by `npm.cmd run delivery:finish`.
Only its exact archive marker permits the native Codex App archive action.
