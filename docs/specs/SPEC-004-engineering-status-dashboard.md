---
spec_id: SPEC-004
title: Local engineering status dashboard
version: 2
status: approved
approved_by: direct_user_reply
approved_at_local: 2026-08-20
---

# SPEC-004 — Local engineering branch map

## Problem

The repository has many TASK and ISSUE worktrees, but their useful state is
distributed across Git, ticket frontmatter, branch names, commits, PR/CI
metadata, and task handoffs. A non-engineer currently has to ask Codex to
reconstruct that state repeatedly.

## Required behavior

1. A double-clickable repository launcher refreshes live evidence and opens one
   self-contained local HTML page.
2. The default view is Traditional Chinese and presents registered worktrees and
   the GitHub default branch as an expandable, top-down tree derived from real
   Git commit ancestry. English branch names and commit details stay behind an
   "engineering details" disclosure.
3. Every node explains in plain Chinese what the branch is for, its current
   state, whether it is safe to use as the starting point for new work, and the
   reason for that decision.
4. A branch is selectable for new work only when its purpose is explicitly
   mapped, its recorded outcome is `COMPLETE` or `VERIFIED`, its evidence is
   complete, its worktree is clean, and its live remote head is synchronized.
   Detached, prunable, stale, partial, failed, blocked, waiting, paused, unknown,
   dirty, or unverified branches fail closed. The live verified GitHub default
   branch is separately selectable as the stable root.
   A snapshot older than 24 hours, more than five minutes in the future, or
   missing its Git ancestry graph disables every new-work choice until refresh.
5. Selecting an eligible node and entering a work description generates a
   copyable Chinese Codex request. The request tells Codex to create a new TASK
   and independent work branch; the page itself never creates or changes work.
6. The collector reads every registered Git worktree without modifying it. A
   source that cannot be proven is `UNKNOWN`; a ticket's explicit `FAIL`,
   `WAITING_DEPENDENCY`, or `VERIFIED` must not be upgraded from clean Git or CI.
7. The generated HTML and machine-readable JSON live outside every Git
   worktree by default, under the user's local application-data directory. A
   refresh must not create a new repository change or status-update commit.
8. Normal task delivery refreshes the projection after handoff and after any
   authorized push. Opening the launcher refreshes again as a fail-safe.
9. The page works offline after generation, requires no database, server,
   account, framework, package installation, or public deployment.
10. The repository delivery protocol requires each new branch to add its plain
    Traditional-Chinese name and purpose to the guide before the normal
    post-handoff and post-push refresh. An unmapped future branch stays visible
    but cannot be selected.

## Plain-language status vocabulary

- `VERIFIED` / `COMPLETE`: green; the explicit ticket or receipt says the work
  passed its own acceptance.
- `IN_PROGRESS`: amber; work is active but has no accepted terminal result.
- `FAIL` / `BLOCKED` / `WAITING_DEPENDENCY`: red; a correction, dependency, or
  authorization is still required.
- `USER_ACTION`: blue; the recorded next gate belongs to the user.
- `UNKNOWN` / `STALE`: gray; current evidence is unavailable or no longer
  matches the branch head.
- `PARTIAL`: amber; the ticket explicitly records incomplete work.
- `PAUSED`: blue; work is deliberately paused without implying failure.
- `SUPERSEDED`: gray; a newer ticket replaced this branch's work.

## Module impact

Create the `engineering-status-dashboard` module. It may depend on Git as a
read-only evidence source, repository ticket files, Node.js standard-library
APIs, and an optional read-only GitHub CLI query. It may not enter `lattice-cli`
because that module forbids Git, process, and network ownership.

## Data, privacy, and security

- Do not render credentials, environment values, command output bodies, or full
  local filesystem paths in the default view.
- Treat repository text as untrusted display data. Insert it through safe text
  APIs; never evaluate it or concatenate it into executable HTML.
- GitHub enrichment is best-effort and read-only. Missing CLI, authentication,
  network, PR, or CI data becomes `UNKNOWN` without blocking local Git results.
- The generated artifact is local and non-authoritative. It grants no execution,
  push, merge, deployment, release, credential, or cleanup authority.

## Edge cases

- Detached worktree, missing upstream, deleted path, unsafe-directory warning,
  malformed/missing ticket, duplicate TASK worktrees, dirty tree, ahead/behind
  branch, unavailable GitHub, and partial collector failure.
- Ticket state conflicts with clean Git or passing CI: preserve the ticket's
  explicit task state and show Git/CI as separate evidence.
- Text containing HTML, script terminators, quotes, Unicode, or long prose must
  display as text and must not execute.
- Several branch names at one commit must be grouped deterministically without
  creating a cycle. A branch with no known Chinese purpose remains visible but
  cannot be selected as a trustworthy starting point.
- A named stable branch outranks a detached/prunable worktree at the same
  commit. Git ancestry read failure makes the snapshot partial, remains visible
  in the page, and cannot produce a recommended branch.

## Acceptance criteria

1. Focused tests create representative temporary Git repositories and prove
   Git-ancestry hierarchy, default-root discovery, fail-closed work eligibility,
   state mapping, remote divergence, safe rendering, output override, and no
   source-tree mutation.
2. A generated snapshot of this repository contains the live TASK-051 and
   TASK-077 worktrees and labels TASK-051 `FAIL` without treating its clean tree
   as completion.
3. The self-contained page opens locally, remains useful at narrow and wide
   widths, expands and collapses a top-down branch tree, selects only eligible
   nodes, and exposes raw technical evidence only on demand.
4. `npm.cmd run check`, the focused dashboard test, and `npm.cmd run verify`
   pass from the identified implementation tree.
5. Final code, security, architecture, and integration reviews have no unresolved
   P0/P1 finding.

## Non-goals

- No LATTICE database writer, event stream, control plane, task scheduler, Git
  mutation, branch creation, commit, push, PR action, merge, deployment, release,
  public hosting, authentication, or multi-user synchronization.
- No claim that the page is a real-time authority. Its generated timestamp and
  incomplete/unknown evidence must stay visible.
- No replacement for tickets, receipts, tests, CI, reviews, or human gates.

## Verification commands

```powershell
npm.cmd run check
node --test test/engineering-status-dashboard.test.js
npm.cmd run status:refresh
npm.cmd run verify
```

## Human decisions

The user approved the local-only implementation on 2026-08-20. The approval
does not include public hosting, PR creation, default-branch merge, deployment,
or release.

## Human acceptance feedback — version 2

On 2026-08-20 the user rejected the card-first V1 because it contained too much
English, did not explain branch purpose, and did not show which branch could
receive new work. The user explicitly requested the expandable top-down branch
tree and selectable new-work starting point defined above. That feedback approves
this version 2 behavior while preserving the same local-only, read-only boundary.
