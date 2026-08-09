---
module_id: passive-supervisor
name: Passive Supervisor
version: 0.1
status: proposed
owner: LATTICE maintainers
last_reviewed: 2026-08-10
---

## Mission

Deterministically classify already-authoritative task, dispatch, worker-session,
and verification evidence as sleeping, pending, running, terminal, or stalled,
then return one passive next-observation recommendation without performing an
effect.

## Non-Goals

- Dispatch, schedule, prioritize, retry, cancel, stop, repair, activate, or
  mutate a task, worker, session, lease, repository, process, or service.
- Own Task Domain transitions, Task Ledger events, worker/session observation,
  verification truth, Writer Lease authority, PostgreSQL persistence, or Git
  state.
- Run an LLM, provider, daemon, poller, database client, process, filesystem,
  network call, MCP server, or adapter.
- Recreate TASK-038 task control, Issue #6/TASK-048 worker observation, the
  existing Orchestrator, or a second truth/writer/gateway.
- Treat a process, lease, timer, diagnostic, or recommendation as task truth.

## Owned Data

- Pure, process-local representations of evidence references, observation
  freshness, passive classification, dependency readiness, stable blocker
  codes, and next-observation recommendations.
- No durable data and no mutation authority. All inputs are projections of
  evidence owned by PostgreSQL/Task Ledger, TASK-038 task control, Issue #6
  worker/session observation, verification, Git, or Writer Lease.

## Public Contracts

- Evaluate one immutable snapshot with an explicit caller-supplied observation
  clock; hidden wall-clock reads are forbidden.
- Preserve task truth separately from passive classification. `STALLED` means
  the evidence is stale, missing, or contradictory; it is never a Task Domain
  transition.
- Return exactly one passive recommendation: wait, observe dispatch, observe a
  worker/session, read verification, recommend durable projection, or sleep.
- Return stable faults for cross-task evidence, impossible ordering, missing
  authoritative task evidence, and contradictory terminal evidence.
- Evaluate dependency readiness from exact stable branch, commit, contract
  version, and focused-verification digest references for TASK-038 and Issue
  #6/TASK-048.

## Invariants

1. The evaluator is deterministic for identical input bytes and has no I/O,
   model, clock, randomness, global state, or side effects.
2. PostgreSQL/Task Ledger task state remains authoritative; Supervisor output
   cannot change or manufacture it.
3. Worker/session evidence is read-only input owned by Issue #6/TASK-048 and
   cannot be inferred from process existence or lease state.
4. Writer Lease evidence is independent authority input and never substitutes
   for task or worker/session evidence.
5. A non-terminal task without sufficiently fresh bound activity becomes a
   `STALLED` observation only; no retry, reassignment, revocation, or repair is
   emitted.
6. Terminal task evidence cannot become completed verification or persisted
   result unless those exact independent evidence references exist.
7. Unknown, missing, cross-bound, future-dated, or contradictory evidence fails
   closed with a stable blocker/fault.
8. The Phase 1 contract exposes no dispatch, scheduling, AI escalation,
   process control, database write, Git write, service activation, or generic
   command surface.
9. Production composition remains blocked until both upstream stable
   checkpoints are exact and machine-verifiable.

## Allowed Dependencies

- Rust standard library only for this isolated Phase 1 contract checkpoint.
- After upstream stabilization, a versioned amendment may allow read-only
  shared representations from the stable TASK-038 and Issue #6 contracts.

## Forbidden Dependencies

- Root workspace registration before the upstream contract gate passes.
- `lattice-orchestrator`, `lattice-postgres-store`, `lattice-writer-lease`,
  concrete adapters, MCP, database/process/filesystem/network/Git clients,
  model SDKs, credentials, or product repositories.

## Failure, Compatibility, And Migration

Version 0.1 is an isolated executable contract checkpoint, not an activated
product module. It deliberately uses no production crate registration or
runtime composition. TASK-038 and Issue #6 adapters may be added only after
both provide exact stable branch/commit/contract references; their mappings
must preserve provider-owned semantics and may not broaden Supervisor
authority. Unknown future status values fail closed until a versioned mapping
is reviewed. Removing the dependency RED gate requires a versioned amendment.

## Acceptance Gates

| Gate | Evidence | Owner | Required for merge |
|---|---|---|---|
| Pure classification | focused event/fault fixtures for sleeping, pending, running, terminal, stalled, and conflict outcomes | Engineering | yes |
| Determinism and no I/O | repeat-evaluation equality plus dependency/source scan | Engineering | yes |
| TASK-038 stable contract | exact branch, 40-hex commit, contract version, and 64-hex focused-verification digest | TASK-038 owner | yes |
| Issue #6 stable contract | exact branch, 40-hex commit, contract version, and 64-hex provider-neutral verification digest | Issue #6 owner | yes |
| Dependency gate | executable RED until both upstream checkpoints are stable | Engineering | yes |
| Runtime composition | PostgreSQL replay and passive loop integration after both stable checkpoints | Integration owner | yes |

## Change Policy

Mission, ownership, classifications, recommendations, fault semantics,
dependency direction, or any new effect requires a versioned amendment,
architecture review, and responsible-user approval. Phase 2 dispatch, AI
scheduling, automatic repair, and service activation require separate tickets
and cannot be added as a 0.1 implementation detail.

## Amendment History

| Version | Date | Decision reference | Summary | Approver |
|---|---|---|---|---|
| 0.1 | 2026-08-10 | GitHub Issue #2 and delegated Phase 1 boundary | Proposed passive-only deterministic Supervisor contract and upstream RED gate | Current user task |
