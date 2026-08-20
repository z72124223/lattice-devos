---
spec_id: SPEC-005
title: Fail-closed engineering delivery finisher
version: 5
status: approved
approved_by: direct_user_reply
approved_at_local: 2026-08-20
---

# SPEC-005 — Engineering delivery finisher

## Problem

LATTICE currently documents that Codex should push an authorized feature branch
and refresh the local engineering map, but nothing executes or verifies those
steps as one operation. A Codex task can therefore stop after reporting its
result, leaving GitHub and the user's local map behind the actual local commit.
The Codex task also remains open even when every delivery gate succeeded.

## Required behavior

1. One repository command finishes an already committed, clean task branch. It
   identifies the current TASK ticket and refuses detached, mismatched, default,
   dirty, malformed, or ambiguous state. Ticket and `PLANS.md` authority are
   read from the captured commit tree, never from a status-hidden working copy.
2. The ticket must declare exactly one machine-readable push policy:
   `authorized_non_force_feature_branch` or `local_only`. Missing or unknown
   policy fails closed.
3. The authorized policy performs only a non-force push of the current named
   feature branch to the ticket's declared remote and canonical repository
   identity. The remote must expose exactly one identical fetch/push endpoint;
   that identity is captured and rechecked. It cannot push another branch, the
   default branch, tags, or a force update. A `local_only` task never pushes.
4. After an authorized push, the command reads the live remote branch head and
   requires exact equality with local `HEAD`, configures the named upstream,
   and verifies the upstream resolves to that same commit. A push error or
   mismatch is a failed delivery, not success.
5. The command refreshes the existing local engineering dashboard after the Git
   decision. When push or verification fails it still attempts a best-effort
   refresh so the page can show the divergence, but the overall command remains
   failed. Generation occurs in a unique external staging directory and only
   replaces a proven app-owned dashboard directory after containment and its
   fixed file set are rechecked. A refresh that returns without both regular
   `index.html` and `status.json` fails and preserves the prior map. Unowned
   files are never removed.
6. The command emits a bounded, single-line-safe terminal result. Success emits
   `LATTICE_DELIVERY_FINISHED=1`; failure emits only the failure marker, a fixed
   error code, and a sanitized one-line explanation. Untrusted input can never
   inject the reserved archive marker into failed output.
7. The ticket must declare `delivery_archive: after_success` or `keep_open`.
   Only a fully successful delivery whose task state is `COMPLETE`, `COMPLETED`,
   or `VERIFIED` and whose policy is `after_success` emits the exact machine
   signal `LATTICE_DELIVERY_READY_TO_ARCHIVE=1`. Failed, blocked, partial,
   paused, or dependency-waiting work may be preserved and mapped but stays open.
8. Repository code cannot archive the Codex App task directly. After seeing the
   success signal, Codex calls the app's native archive-task action as its final
   action. Failure, interruption, `keep_open`, or a missing signal leaves the
   task visible for diagnosis.
9. The command never creates commits, edits handoffs, chooses authorization,
   opens PRs, merges, deploys, releases, deletes worktrees, or changes credentials.
   Codex must finish tests, durable handoff, and the logical commit before invoking it.
10. After dashboard refresh, the command rechecks the current branch, exact
    local head, clean worktree, and (when pushed) live remote head. A concurrent
    local or remote change prevents success and archival.
11. The current branch must be exactly `feature/task-nnn-*` or
    `feature/issue-nnn-*`, with a lowercase hyphenated slug. A task branch's
    `nnn` must equal its one unique committed terminal TASK ticket ID. Any
    declared TASK `depends_on` identities must each resolve exactly once from
    that captured tree and already be successfully terminal. An issue branch's
    `nnn` must equal one unique committed
    `docs/issues/ISSUE-nnn-*.md` `issue_id`, whose `branch` equals the current
    branch exactly. Both evidence types must be terminal and declare the named
    remote, credential-free repository identity, push policy, and archive
    policy. GitHub branch metadata alone is never delivery evidence.
    `PLANS.md` remains a planning index, not a delivery lock across parallel
    worktrees, and is not consulted for delivery authorization.

## Module impact

Create `engineering-delivery-finisher` as a separate module. The existing
`engineering-status-dashboard` remains read-only; the new module alone owns the
bounded delivery sequence and delegates projection generation to the dashboard's
public exporter command.

## Data, privacy, and security

- Ticket text and Git output are untrusted inputs. Branch, remote names, and
  captured endpoint URLs are passed as process arguments without a shell.
- Client-side repository Git hooks are disabled for every finisher Git command;
  the push additionally uses `--no-verify`.
- The exact authorization token is repository evidence of previously granted
  scope; it does not grant default-branch, merge, deployment, or release authority.
- No environment dump, remote URL credentials, command output body, or full
  local path is emitted in terminal diagnostics.
- Credential-bearing remote URLs and URL query/fragment components are rejected
  before any live network Git command.
- A failed dashboard refresh, remote lookup, or Git operation prevents the
  archive-ready signal.
- The dashboard output must be disjoint from the source repository (neither
  inside it nor its ancestor), including through an existing filesystem link.
  A fixed ownership marker is required after the first successful finisher run;
  the legacy exact `index.html` plus `status.json` pair migrates safely.

## Edge cases

- No upstream yet; existing synchronized upstream; local branch ahead; remote
  branch moved independently; remote branch absent; push rejection.
- Fetch/push endpoint split, duplicate remote URLs, unauthorized repository
  identity, remote config mutation, or live default-branch mutation.
- Repository pre-push or reference hooks cannot add delivery side effects.
- Detached HEAD, default branch, dirty/untracked file, duplicate TASK tickets,
  malformed frontmatter, branch mismatch, missing remote, unknown policies.
- Duplicate ISSUE identities, issue-number/branch mismatch, missing committed
  ISSUE evidence, or non-terminal ISSUE evidence.
- Dashboard refresh succeeds after a push failure; the command still reports
  failure and forbids archiving.
- `skip-worktree` or `assume-unchanged` cannot substitute ticket/PLANS
  authorization because the captured commit tree is authoritative.
- A failed preflight plus an output junction swap cannot direct best-effort
  refresh writes into the source repository.
- An arbitrary existing directory or a repository ancestor passed as `--output`
  is rejected without refresh; existing unowned sentinel data is preserved.
- A zero-output refresh exit 0 is not success and cannot replace the prior map
  or emit archive permission.
- Local or remote state changes while refresh is running: the final state gate
  fails and preserves the task window.
- Two tasks finish close together; each refresh remains a disposable
  last-observed projection and neither process gains authority from the page.

## Acceptance criteria

1. Temporary real Git repositories prove local-only never pushes, authorized
   delivery pushes without force, verifies exact remote/upstream equality, and
   divergence/default/detached/dirty/malformed cases fail closed.
2. Tests prove refresh is attempted after both success and post-preflight Git
   failure, while refresh failure prevents archive readiness.
   Tests also prove output containment and the post-refresh local/remote gate.
3. Project governance requires the current ticket's two delivery policies and
   requires Codex to use the finisher plus the app archive action instead of a
   manual ordinary-task push.
4. `npm.cmd run check`, focused finisher tests, and `npm.cmd run verify` pass.
5. Code/security and architecture review have no unresolved P0/P1 finding;
   exact integration verification against the GitHub default target is recorded.
6. Focused evidence proves terminal ISSUE-007 and ISSUE-008 records can deliver
   only their exact branches, while duplicate identities, branch-number
   mismatches, arbitrary feature branches, non-terminal evidence, unanchored
   GitHub-only branches, and TASK-number collisions fail closed.

## Non-goals

- Automatic commit creation, automatic authorization inference, default-branch
  push/merge, PR actions, deployment, release, hosting, scheduler/daemon, Git
  hook, background polling, or archival of a failed task.
- Replacing Git, tickets, tests, CI, handoff, LATTICE receipts, or human authority.

## Amendment history

- v2 removed a redundant receipt write and bound delivery to the current
  terminal TASK after independent security review.
- v3 binds committed authority and remote endpoint identity, rejects
  credential-bearing endpoints, stages dashboard publication outside source,
  rechecks live default/config state, and makes the named upstream observable
  to the engineering map.
- v4 preserves the TASK grammar and admits legacy `feature/issue-nnn-*` only
  through unique, committed terminal ISSUE evidence; it never infers issue
  authority from GitHub metadata or collides with the TASK namespace.
- v5 makes committed terminal ticket/evidence identity the parallel-delivery
  authority. Declared TASK dependencies must be verifiably complete in the
  same captured tree; PLANS is a non-authoritative planning index.

## Verification commands

```powershell
npm.cmd run check
node --test test/engineering-delivery-finisher.test.js
node --test test/project-governance-check.test.js
npm.cmd run verify
git diff --check
```

## Human decisions

On 2026-08-20 the user explicitly approved the proposed one-command finisher
and then added automatic archival of the current Codex task after successful
completion. This approval covers a non-force push of this feature branch and
the final app archive action. It does not authorize a PR, default-branch merge,
deployment, release, or archival after failure.

Version 2 removes the initially proposed extra delivery-receipt file after
independent review proved that a final post-gate write created an unnecessary
filesystem-link race. GitHub plus the refreshed engineering map remain the
requested durable views; the exact process success marker alone permits native
task archival. This is a security-preserving simplification within the user's
approved lightweight outcome.
