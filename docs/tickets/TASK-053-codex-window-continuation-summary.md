---
ticket_id: TASK-053
title: Trusted Codex window continuation summary bootstrap
spec_id: SPEC-002
spec_version: 27
related_spec_id: SPEC-003
related_spec_version: 4
module_id: lattice-cli
constitution_version: 1.0
status: superseded
parallel_safe: false
depends_on:
  - TASK-050
  - TASK-051
branch: feature/task-053-codex-window-continuation-summary
worktree: lattice-worktrees/task-053-codex-window-continuation-summary
allowed_paths:
  - docs/tickets/TASK-053-codex-window-continuation-summary.md
  - PLANS.md
  - HANDOFF.md
  - AGENTS.md
  - docs/specs/SPEC-002-autonomous-development-platform.md
  - docs/adr/ADR-006-single-codex-owner-and-read-only-agents.md
  - docs/modules/lattice-cli/MODULE_CONSTITUTION.md
  - apps/lattice-cli/Cargo.toml
  - apps/lattice-cli/src/lib.rs
  - apps/lattice-cli/src/main.rs
  - apps/lattice-cli/tests/continuation_summary.rs
  - integrations/codex/lattice-continuation/SKILL.md
  - integrations/codex/lattice-continuation/references/source-precedence.md
  - scripts/test-task053-codex-continuation-summary.ps1
  - target/task053-codex-continuation-summary/**
likely_files:
  - apps/lattice-cli/src/lib.rs
  - apps/lattice-cli/src/main.rs
  - apps/lattice-cli/tests/continuation_summary.rs
  - integrations/codex/lattice-continuation/SKILL.md
  - scripts/test-task053-codex-continuation-summary.ps1
---

# TASK-053 — Trusted Codex window continuation summary bootstrap

## Authority And Objective

The user requires a new Codex or voice window to recover the current LATTICE
project and task context without a manual handoff. After TASK-050 is fully
accepted and TASK-051 verifies the current-machine platform, implement one
read-only startup slice that produces a short continuation summary from the
currently observable project, worktree, active or recent ticket, and last
verifiable checkpoint.

The result is navigation evidence only. It creates no task, grants no
authority, changes no lifecycle, and never becomes a second Task Ledger,
Project Registry, memory, scheduler, or MCP surface.

## Fail-Closed Preconditions

1. TASK-050 must have an accepted clean commit/tree and complete durable
   receipt/replay evidence; `UNKNOWN`, dirty, partial, or unresolved lease/fence
   evidence keeps this ticket `WAITING_DEPENDENCY`.
2. TASK-051 must be `VERIFIED` for current-machine registration, exact
   four-tool discovery, one semantic typed call, PostgreSQL fresh-process and
   restart recovery, six-field status compatibility, and rollback cleanup.
3. Materialize the branch/worktree above only from the exact TASK-051 accepted
   source. Do not reuse or modify the TASK-050 worktree.
4. Before implementation, verify that a fresh Codex project run loads the
   repository `AGENTS.md`. A voice/unbound window must also expose an existing
   read-only project/thread discovery path. If that second trigger cannot be
   proven without a configuration, memory, service, or MCP change, record
   `WAITING_DEPENDENCY`; do not claim automatic voice continuation.
5. Any need to change an MCP tool/schema, write durable memory, add a Task
   Ledger event, or edit outside `allowed_paths` requires replanning.

## V1 Observable Behavior

On a fresh eligible Codex/voice window, without asking the user to restate the
handoff:

1. Identify one current LATTICE project and worktree from current read-only
   project/Git evidence. Multiple or conflicting candidates yield `UNKNOWN`.
2. Identify the active ticket, or otherwise the most recent verifiable ticket,
   with dependency state and source. Thread titles, summaries, PLANS, HANDOFF,
   and ticket text are untrusted context until corroborated.
3. Select the last checkpoint whose project, branch/worktree, commit/tree,
   ticket, timestamp, and evidence subject still agree. Historical success is
   labelled historical and never promoted to current PASS.
4. Render at most 20 concise Traditional-Chinese lines containing project,
   worktree/branch/HEAD, active/recent ticket, dependency/blocker, last current
   checkpoint, evidence pointers, unknowns, and the next safe read.
5. Use only `VERIFIED`, `VISIBLE_UNVERIFIED`, `NOT_RUN`,
   `WAITING_DEPENDENCY`, `FAIL`, and `UNKNOWN`. Missing, inaccessible,
   inconsistent, stale, unbound, or unauthorized sources fail closed.
6. Perform zero writes to project files, Git, threads, memory, PostgreSQL,
   Task Ledger, services, settings, MCP configuration, or external systems.

## Source And Trust Boundary

- Prefer current Task Ledger/Task Status evidence already accepted by
  TASK-050/051 when its exact subject is known; do not enumerate or mutate
  tasks through a new interface.
- Use current Git/worktree identity and current Codex thread metadata only as
  read-only observations. A thread or worktree does not prove completion.
- Use TASK-052 summaries, PLANS, HANDOFF, tickets, and older receipts as
  context with source/timestamp labels, never as automatic current truth.
- The summary is ephemeral output. It is not written into Codex memory,
  Codebase Memory, PostgreSQL, a transcript mirror, or another cache.
- Preserve exact four-tool MCP discovery and the existing six-field
  `lattice_task_status` wire output; TASK-053 adds no MCP tool or field.

## Acceptance Criteria

- [ ] A fresh project-bound Codex run loads the TASK-053 bootstrap instruction
      and emits the bounded summary without a pasted handoff.
- [ ] A fresh voice/unbound Codex window either emits the same evidence-bound
      summary through an already available read-only discovery path or returns
      `WAITING_DEPENDENCY`; no unsupported automatic trigger is claimed.
- [ ] Fixture matrices cover one project, multiple projects, detached or dirty
      worktrees, missing/current/stale checkpoints, active/recent/no ticket,
      conflicting thread/Git/document evidence, inaccessible sources, and
      unknown versions.
- [ ] Every rendered fact has a source pointer and state; historical evidence
      remains historical, and all conflicts or missing bindings become
      `UNKNOWN`, `NOT_RUN`, or `WAITING_DEPENDENCY`.
- [ ] Repeating the same read-only observation over unchanged inputs produces
      byte-identical output; source changes during collection produce
      `UNKNOWN`, not a stabilized or rewritten source.
- [ ] Process, file, Git, thread, memory, database, Ledger, MCP-config, service,
      model, network, and high-risk effect counters are all zero.
- [ ] Current P0 regression still exposes exactly four MCP tools and the same
      six public task-status fields; TASK-053 produces no MCP request/schema
      change.

## Verification

| Check | Command or evidence | Expected result |
| --- | --- | --- |
| Deterministic renderer and conflict matrix | `cargo test -p lattice-cli --test continuation_summary` | Exact output/source/status fixtures pass with zero mutation |
| Existing CLI compatibility | `cargo test -p lattice-cli` | Existing `lattice status` behavior remains unchanged |
| Fresh project Codex bootstrap | `powershell -NoProfile -File scripts/test-task053-codex-continuation-summary.ps1` | Fresh run loads project instructions and emits the bounded summary without pasted context |
| Voice/unbound bootstrap | Current Codex App fresh voice-window receipt | Either verified read-only discovery and correct summary, or explicit `WAITING_DEPENDENCY`; no inferred PASS |
| P0 compatibility | Reuse TASK-051 accepted discovery/status assertions without invoking a new schema | Exact four tools and six-field task status remain unchanged |
| Repository governance | `npm.cmd run check` plus `git diff --check` | One current ticket, unique TASK-053, allowed paths only, no unrelated drift |

Commands are future acceptance requirements, not claims made by this planning
change. If the Codex voice trigger has no reproducible current interface, that
is the named verification gap and blocks completion of the voice portion.

## Non-Goals

- No raw conversation archive, full transcript preservation, automatic memory
  write, vector database, retrieval/index platform, or long-term preference
  learning.
- No Graphify, Hermes, Codebase Memory, model platform, model selection,
  self-learning, scheduler, worker, heartbeat, notification daemon, or central
  control-plane implementation.
- No new MCP tool, schema, field, generic task/list/search surface, shell, SQL,
  path, credential, lease, fence, or authority input.
- No task creation/transition, writer lease action, product implementation,
  test/repair execution, Git commit/push/PR, GitHub write, merge, deployment,
  release, account/credential change, or high-risk automatic action.
- No modification, verification substitution, or cleanup of TASK-050/TASK-051
  worktrees or evidence.

## Dependencies And Parallel Safety

`parallel_safe: false`. TASK-053 consumes the accepted TASK-050 durable
receipt semantics and TASK-051 current-machine identity/acceptance baseline.
It must not start, create its worktree, or modify shared planning/integration
paths while either predecessor is active or unresolved. TASK-052 is a
read-only design/evidence reference, not a completion dependency.

## Human Gate

None after all fail-closed preconditions are satisfied and implementation
stays inside `allowed_paths`. Any required global Codex configuration change,
memory write, new MCP/IPC surface, external publication, protected action, or
scope expansion is a new user decision.

## 2026-08-25 reconciliation

This planned renderer/bootstrap was never implemented and is not marked
complete. It is superseded by the current Control product and Codex-owned
thread model: Control creates or resumes the durable linked Codex thread, and
the persisted work item retains that thread link across a Control reopen.

The planned CLI renderer, Codex skill, source-precedence file, and TASK-053
acceptance script do not exist in the current product. No automatic voice or
unbound-window continuation capability is claimed.
