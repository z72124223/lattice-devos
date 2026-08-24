---
ticket_id: TASK-048
spec_id: GITHUB-ISSUE-6
spec_version: 2026-08-09
module_id: lattice-contracts
constitution_version: 1.12
status: completed
parallel_safe: false
depends_on: []
allowed_paths:
  - crates/lattice-contracts/src/lib.rs
  - crates/lattice-contracts/src/worker_observation.rs
  - crates/lattice-contracts/tests/worker_observation_contracts.rs
  - docs/modules/lattice-contracts/MODULE_CONSTITUTION.md
  - docs/tickets/TASK-048-worker-observation-contract.md
likely_files:
  - crates/lattice-contracts/src/worker_observation.rs
  - crates/lattice-contracts/tests/worker_observation_contracts.rs
branch: feature/task-048-worker-observation-contract
---

## Objective

Freeze one provider-neutral, I/O-free observation contract for GitHub Issue #6
before any PostgreSQL persistence, adapter, runtime composition, or MCP tool is
added. Codex is a first consumer, not the identity of the model.

## Acceptance Criteria

- [x] Worker Provider, Worker Instance, Work Session, Activity Event, Task
      Binding, and Process Binding are independently typed and bounded.
- [x] Process state, work-session state, LATTICE task projection, and
      Writer-Lease/runtime-admission observation remain separate and cannot be
      promoted into one another.
- [x] LATTICE-managed, provider-managed, user-managed, discovered-only, and
      unknown ownership plus managed/formal/process-only/unobservable
      visibility are represented explicitly.
- [x] Observation source, confidence, freshness, and time remain explicit;
      process discovery cannot claim provider-reported or verified confidence.
- [x] Process-only discovery cannot claim a task binding, session progress,
      session/task activity meaning beyond process lifecycle, or writer
      authority.
- [x] The same public contract represents Codex, PowerShell, and a non-Codex
      verification/tool provider.
- [x] Query representation is closed to read-only worker/session list/status;
      pause/resume/kill/cancel and writer-lease acquisition are unrepresentable.
- [x] No observation or activity type contains a command, shell history,
      environment, prompt, conversation, raw stderr, credential, secret,
      screen/OCR/keylogging, or arbitrary-path field.

## Non-Goals

PostgreSQL schema/migrations, durable repositories, provider adapters, Codex
app-server event mapping, PowerShell/WSL supervision, MCP registration,
runtime/orchestrator composition, process control, cancellation, services,
deployment, and full Issue #6 acceptance.

## Module And Constitution Constraints

`lattice-contracts` 1.12 owns only immutable I/O-free representations. Task
Domain retains task transitions, Task Ledger/PostgreSQL retains durable task
truth, Writer Lease retains authority, and a future observation owner retains
event/projection persistence. Carrying a task projection or lease head is
read-only structural evidence, not currentness or authority.

## Dependencies And Overlap

The pure contract slice is based on stable TASK-038 checkpoint
`512732d5b71a5d373363b77bb23a29e4a8ae3b1b` and changes none of TASK-038's
active root Cargo, runtime, orchestrator, MCP, PostgreSQL, task-control, lease,
`PLANS.md`, or `HANDOFF.md` paths. It is marked non-parallel-safe because it
changes a shared public contract; runtime/tool integration remains blocked on
TASK-038's next clean stable checkpoint and an explicit contract handoff.

## TDD Behaviors

1. RED/GREEN provider/instance/session/activity/task/process construction and
   exact cross-binding.
2. RED/GREEN independent process/session/task/authority states, including a
   running process with idle or stale session evidence.
3. RED/GREEN honest degradation for process-only and unobservable sessions.
4. RED/GREEN evidence-source/confidence compatibility and process-only
   lifecycle activity without session-progress claims.
5. RED/GREEN Codex, PowerShell, and verification-provider neutrality.
6. RED/GREEN closed read-only list/status queries and bounded cursors/pages.

## Verification

| Check | Command or service | Expected evidence |
|---|---|---|
| Focused contract | `cargo test -p lattice-contracts --test worker_observation_contracts --locked` | all worker observation cases pass |
| Contract regression | `cargo test -p lattice-contracts --locked` | all contract tests pass |
| Quality | `cargo fmt --all -- --check` and scoped strict Clippy | exit 0 |
| Governance | `npm.cmd run check` and `git diff --check` | one current task retained; unique TASK-048; clean diff |

## Human Gate

None for this pure local contract checkpoint: GitHub Issue #6 is the current
user-authoritative behavior specification. PostgreSQL persistence, TASK-038
runtime/MCP integration, push, merge, deployment, and service activation remain
separate work.
