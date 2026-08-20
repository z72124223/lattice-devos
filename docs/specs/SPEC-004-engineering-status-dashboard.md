---
spec_id: SPEC-004
title: Local engineering status dashboard
version: 1
status: approved
approved_by: direct_user_reply
approved_at_local: 2026-08-20
---

# SPEC-004 — Local engineering status dashboard

## Problem

The repository has many TASK and ISSUE worktrees, but their useful state is
distributed across Git, ticket frontmatter, branch names, commits, PR/CI
metadata, and task handoffs. A non-engineer currently has to ask Codex to
reconstruct that state repeatedly.

## Required behavior

1. A double-clickable repository launcher refreshes live evidence and opens one
   self-contained local HTML page.
2. The page presents a plain-language current focus, counts, filters, and one
   readable card per branch/worktree. Technical evidence is available behind a
   disclosure control.
3. Each card distinguishes TASK/ISSUE identity, ticket state, last explicit
   terminal outcome, worktree cleanliness, remote synchronization, last commit,
   PR/CI evidence when available, user action, and the next known step.
4. The collector reads every registered Git worktree without modifying it. A
   source that cannot be proven is `UNKNOWN`; a ticket's explicit `FAIL`,
   `WAITING_DEPENDENCY`, or `VERIFIED` must not be upgraded from clean Git or CI.
5. The generated HTML and machine-readable JSON live outside every Git
   worktree by default, under the user's local application-data directory. A
   refresh must not create a new repository change or status-update commit.
6. Normal task delivery refreshes the projection after handoff and after any
   authorized push. Opening the launcher refreshes again as a fail-safe.
7. The page works offline after generation, requires no database, server,
   account, framework, package installation, or public deployment.

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

## Acceptance criteria

1. Focused tests create representative temporary Git repositories and prove
   collection, state mapping, remote divergence, safe rendering, partial-failure
   behavior, output location override, and no source-tree mutation.
2. A generated snapshot of this repository contains the live TASK-051 and
   TASK-077 worktrees and labels TASK-051 `FAIL` without treating its clean tree
   as completion.
3. The self-contained page opens locally, remains useful at narrow and wide
   widths, filters cards without navigation, and exposes technical evidence only
   on demand.
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
