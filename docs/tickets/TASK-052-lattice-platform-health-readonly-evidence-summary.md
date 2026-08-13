---
ticket_id: TASK-052
title: LATTICE platform health read-only evidence summary
spec_id: SPEC-003
spec_version: 4
module_id: latticed
constitution_version: 1.4
status: ready
parallel_safe: true
depends_on: []
allowed_paths:
  - docs/tickets/TASK-052-lattice-platform-health-readonly-evidence-summary.md
  - target/task052-platform-health-readonly/<snapshot_id>/**
likely_files:
  - target/task052-platform-health-readonly/<snapshot_id>/summary.md
  - target/task052-platform-health-readonly/<snapshot_id>/evidence-index.json
branch: none
---

# TASK-052 — LATTICE platform health read-only evidence summary

## Objective

Produce a compact, resumable, read-only snapshot that lets a voice coordination task or a newly opened Codex task determine the current state of active LATTICE work and core platform gates, the last directly verified evidence, and the next safe observation. The snapshot observes existing state only; it must not change LATTICE runtime, MCP configuration, PostgreSQL or Task Ledger state, Git/worktree state, task/thread state, TASK-050, or TASK-051.

This ticket has no dependency on TASK-050 or TASK-051 and creates no dependency for them. It may run concurrently because it owns no shared mutable path, invokes no product behavior, and performs no state transition. A concurrent change observed mid-snapshot must be reported as `UNKNOWN`, not stabilized or retried by mutating the system.

## Historical Evidence

One zero-impact snapshot was produced on 2026-08-13 as
`task052-20260813T014939+0800`. The snapshot artifacts themselves were
verified, while their overall platform conclusion was `FAIL`; they are
historical context only and cannot establish current platform health. This
ticket remains `ready` for a fresh read-only snapshot.

## Scope and constitution boundaries

- Observe the existing SPEC-003 v4 platform surface and the `latticed` 1.4 constitution without changing either.
- Treat Task Ledger 2.1, postgres-store 1.6, and orchestrator-runtime 2.4 as read-only evidence sources only. If an accepted active task changes these versions, record the observed version and classify compatibility as `UNKNOWN`; do not infer it.
- Preserve One Gateway, One Truth, One Writer, lease/fence, credential, rollback, and closed MCP-surface boundaries.
- The only writable outputs are the `allowed_paths` above. No branch, worktree, commit, test fixture, runtime configuration, or service process may be created.

## Status vocabulary

- `VERIFIED`: a current read in this snapshot directly proves the stated observation and records source identity, timestamp, and evidence hash.
- `VISIBLE_UNVERIFIED`: an object is visible, but semantic health was not or could not be established without a prohibited action.
- `NOT_RUN`: an authorized read-only observation was not attempted in this snapshot.
- `WAITING_DEPENDENCY`: a named source, capability, authority, or prerequisite needed for observation is unavailable.
- `FAIL`: a read-only observation was attempted and returned an explicit failure or contradicted an exact accepted requirement.
- `UNKNOWN`: evidence is missing, ambiguous, changes during observation, or cannot be refreshed without mutation.

Historical snapshots are context only. They must be labelled `historical_evidence` with their original timestamp and identity; they never make a current row `VERIFIED` or current PASS.

## Fail-closed preconditions

Before collecting evidence, the observer must record a unique `snapshot_id`, start time, observer/Codex identity, repository root, current read-only capability set, and the exact output directory under `target/task052-platform-health-readonly/<snapshot_id>/`.

The run must stop or downgrade affected rows according to these rules:

1. Only read operations are permitted. If a required tool can start/stop/reload a process, invoke an MCP tool, update a thread, modify Git refs/index/worktrees, change configuration, acquire a writer lease, or write to PostgreSQL, do not call it; classify the row `WAITING_DEPENDENCY` or `UNKNOWN`.
2. Thread evidence may use list/read operations only. No send, handoff, archive, pin, rename, fork, or task-state update is permitted.
3. Git evidence may use working-tree/ref/object inspection already present locally. No fetch, pull, checkout, switch, branch, worktree, add, commit, reset, clean, stash, tag, push, or remote write is permitted.
4. MCP evidence may inspect discovery already available to the current Codex process. If discovery would require registration changes, a new config generation, server launch, reload, or a real tool invocation, record `NOT_RUN`, `WAITING_DEPENDENCY`, or `UNKNOWN`. Tool visibility is not semantic success.
5. Process evidence may inspect existing process identity, parentage, executable path/hash, and listeners only. No signal, stop, kill, start, attach, containment change, or lifecycle threshold/leak inference is permitted.
6. PostgreSQL evidence requires an existing explicitly read-only connection path and a read-only transaction. No DDL, DML, migration, lock/lease acquisition, sequence use, advisory lock, restart, database creation, configuration change, or cleanup is permitted. Without a provably read-only path, PostgreSQL rows are `WAITING_DEPENDENCY`.
7. Do not read or emit secrets. Redact credentials, environment values, connection strings, tokens, and sensitive command lines before hashing or writing evidence.
8. If a source changes during collection, preserve both observations, mark the affected row `UNKNOWN`, and identify the concurrent-change boundary. Do not make the source stable.

## Required evidence inventory

The snapshot must include, at minimum:

1. **Active work:** current visible state of TASK-050, TASK-051, and every other active LATTICE task/thread found by the read-only inventory; exact worktree/branch/HEAD/dirty state when locally observable; last current verification receipt and its source timestamp.
2. **MCP registration/discovery gate:** current-process registration visibility, registered binary identity when already observable, discovered tool names/schemas, and whether discovery is current, visible-only, unavailable, or not run. No tool invocation is part of TASK-052.
3. **Core live-acceptance gates:** status of typed semantic invocation, PostgreSQL durable write/fresh read/restart recovery, exact four-tool discovery, and six-field `lattice_task_status` wire regression. TASK-052 reports existing evidence only and never performs these gates.
4. **Runtime identity and health:** currently observable LATTICE process count/parentage/binary hashes/listeners and PostgreSQL server/database identity through permitted reads. Process existence or a listening port is `VISIBLE_UNVERIFIED`, not platform health.
5. **Evidence lineage:** exact source pointer, timestamp, commit/tree/binary/config/process/database identity when present, evidence hash, current-versus-historical classification, and any mismatch or missing link.

Any inventory item that cannot be refreshed immediately by an allowed read must be `UNKNOWN`, `NOT_RUN`, or `WAITING_DEPENDENCY`. Do not copy a prior PASS into the current-state column.

## Output summary format

`summary.md` must remain concise and independently understandable:

1. **Snapshot header:** `snapshot_id`, start/end time with timezone, observer identity, repository root, and read-only capability limits.
2. **Overall platform state:** one of `VERIFIED`, `VISIBLE_UNVERIFIED`, `FAIL`, `UNKNOWN`, or `WAITING_DEPENDENCY`, computed fail closed from the required gates; never infer overall PASS from partial visibility.
3. **Active work table:** `work_id | owner/thread | branch/worktree | state | last_current_evidence_at | evidence_ref | blocker_or_next_read`.
4. **Platform gate table:** `gate | state | observed_at | subject_identity | evidence_ref | historical_context | gap_or_next_safe_read`.
5. **Zero-impact receipt:** allowed reads used, prohibited actions count (must be zero), repository writes (only the two snapshot artifacts), runtime/Task Ledger mutations attributable to TASK-052 (must be zero), and redaction result.
6. **Unknowns and handoff:** every `UNKNOWN`, `NOT_RUN`, or `WAITING_DEPENDENCY` row with the exact missing evidence and no invented remediation claim.

`evidence-index.json` must contain versioned, deterministic keys for the same rows and hashes, but no raw secret-bearing logs. It is an index, not a new LATTICE product schema or Task Ledger event.

## Acceptance criteria

- [ ] TASK-052 collision check records no pre-existing ticket or ref owner before this ticket is accepted.
- [ ] The snapshot writes only `summary.md` and `evidence-index.json` beneath one allowed `snapshot_id` directory; no existing file is changed.
- [ ] TASK-050 and TASK-051 are observed without file, thread, task, worktree, service, MCP, database, lease, or ledger mutation and without becoming dependencies of TASK-052.
- [ ] Every required active-work and platform-gate row has a current status, observation timestamp, source/evidence pointer, and explicit gap; unrefreshable items use `UNKNOWN`, `NOT_RUN`, or `WAITING_DEPENDENCY`.
- [ ] Historical evidence is separated from current state and never promoted to current PASS.
- [ ] MCP visibility is separated from registration identity, tool discovery, semantic invocation, and live acceptance.
- [ ] Process/listener visibility is separated from executable identity and semantic health; PostgreSQL visibility is separated from durable write/replay acceptance.
- [ ] The zero-impact receipt shows only allowlisted read operations, zero prohibited actions, zero TASK-052-attributable runtime/Task Ledger mutations, and no secret disclosure.
- [ ] A new Codex task can read only the two snapshot artifacts and identify the current state, last verified evidence, unresolved gaps, and next safe read for each listed gate.

## Read-only verification matrix

| Check | Permitted method | Expected evidence |
| --- | --- | --- |
| Ticket/branch collision | Local path and local ref inspection | No existing TASK-052 owner; timestamped result |
| Active work inventory | Thread list/read plus local Git object/status/worktree inspection | Current rows with identities and dirty-state evidence; no writes |
| MCP inventory | Current-process discovery inspection only | Registration/discovery state or explicit `NOT_RUN`/`UNKNOWN`; no invoke/start/reload |
| Process inventory | Existing OS process/listener inspection | Timestamped PID/parent/path/hash/listener rows; no lifecycle action |
| PostgreSQL inventory | Existing read-only connection and read-only transaction only | Server/database identity and read-only health evidence, or `WAITING_DEPENDENCY` |
| Zero impact | Operation audit plus pre/post read-only observations | Only allowed artifact writes; zero attributable runtime, Git, thread, service, or Ledger mutation |
| Summary usability | Fresh Codex reads the two artifacts without querying live systems | Every work/gate state, evidence pointer, unknown, and next safe read is recoverable |

## Non-goals

- No LATTICE service, MCP server/client, PostgreSQL, worker, scheduler, tunnel, model, Hermes, Graphify, Codebase Memory, or external-provider invocation.
- No real typed MCP call, delivery run, task submit/status call, database write, restart, replay, migration, lease/fence action, cleanup, or machine acceptance. Those remain owned by their existing tickets, including TASK-051.
- No TASK-050/TASK-051 implementation, verification, state transition, blocker, duplicate acceptance, or file access beyond read-only evidence explicitly exposed by the inventory sources.
- No code, script, configuration, existing documentation, specification, ticket, module constitution, plan, handoff, branch, worktree, commit, tag, GitHub object, deployment, or release change.
- No monitoring daemon, recurring automation, notification service, dashboard, new MCP schema/tool, Task Ledger event, or durable platform state.
- No repair or recommendation execution. The summary may name the next safe read or existing owning ticket only.

## Human gate

None for a run that remains inside the exact read-only and output scopes above. Stop and request a new decision before any operation that would mutate a source, start or stop a component, invoke product behavior, expose credentials, or write outside the two snapshot artifacts.
