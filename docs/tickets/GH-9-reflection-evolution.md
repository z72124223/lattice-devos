---
ticket_id: GH-9
module_id: task-domain
---

# GH-9 Reflection Evolution — First Minimal Slice

- GitHub issue: <https://github.com/z72124223/lattice-devos/issues/9>
- Canonical key: `GH-9-reflection-evolution`
- Base: `2b424ec9a5401a6fbdc4f37d3d401592331afca0`
- Branch: `feature/gh-9-reflection-evolution`
- Publication: development checkpoint authorized at
  `2026-08-11T01:58:40+08:00` (`Asia/Taipei`); final local completion and
  feature-branch publication authorized on `2026-08-21` for this branch only,
  with no PR, merge, deployment, or release authorization

## Goal

Keep the authoritative core Task lifecycle terminal after successful Codex,
verification, Git, and PostgreSQL replay while recording Reflection as an
independent append-only projection. Hermes receives only bounded typed history
and candidate ports; it cannot mutate core transitions or original events.

## Scope

- Separate typed Reflection states: `REFLECTION_PENDING`,
  `REFLECTION_FAILED`, `RETRY_PENDING`, and `DEGRADED`.
- Append-only failure, rejection, Hermes-failure, claim, and candidate events.
- Deterministic queue, claim, bounded history, and fresh-process replay.
- Reuse the existing PostgreSQL-backed Task Ledger when it preserves the core
  projection and does not require a new physical schema.
- No MCP expansion, Hermes execution, unattended supervisor, model router,
  TASK-037 formal run, publication, or deployment.

## Architecture invariants

1. `TaskState::Completed` and its result/core-head projection are immutable
   under every Reflection append.
2. Reflection events never use `STATE_TRANSITION`: queue admission is an
   immutable `EFFECT_INTENT`, claims and candidates are immutable
   `EVIDENCE_RECORDED` events, and failure/retry/degraded receipts are immutable
   `EFFECT_OUTCOME` events. The existing Task Ledger is the only writer.
3. The core replay and Reflection replay are independent projections over one
   verified append-only stream.
4. The Hermes-facing port exposes only bounded history and closed candidate
   appends. It exposes no SQL, database client, path, raw prompt, stderr,
   credential, update, delete, or core-transition operation.
5. Queue and claim operations are deterministic and idempotent by exact command
   identity. They do not invoke Codex, verification, Git, or Hermes.
6. Only a core `COMPLETED` Task enters the queue/claim/candidate/retry lane.
   `TASK_FAILURE` and fixed-verifier `OUTPUT_REJECTED` records on a failed core
   are terminal, read-only Reflection history: they cannot claim, retry,
   degrade, append a candidate, or revive the core Task.
7. Authorized history is a typed keyset page. Its exclusive sequence cursor,
   bounded limit, returned events, next cursor, exact core anchor, and current
   journal head are committed into the page digest; candidate append replays
   the same page before accepting that digest.
8. This slice is a known-Task lane only. It adds no cross-Task discovery,
   `claim_next`, unattended worker, composition hook, MCP writer, or automatic
   production caller. A bounded caller must already hold the exact Task
   binding and invoke the public typed ports explicitly.

## Workflow ledger

| Step | RED evidence | GREEN evidence | Status |
| --- | --- | --- | --- |
| Core completion survives Hermes failure | `reflection_tail_does_not_rewrite_completed_core_projection` failed because the projection had no independent core anchor: the current stream head moved from `0d6c05…` to `eb2291…`. | A new typed `core_head_digest` is derived from the verified receipt of the last legal core event and stays stable, while the existing public `ledger_head_digest` deliberately remains the current append-only stream head; the exact test passes. | Green |
| Hermes failure is durably persisted | The base had no typed Reflection projector: a generic tail could exist, but no closed `REFLECTION_FAILED` receipt could be replayed. | `hermes_failure_replays_independently_from_completed_core` replays the exact claimed `HERMES_FAILURE` event and leaves the core result/head unchanged. | Green |
| Fresh replay returns core + Reflection | The base fresh-load surface returned only `TaskLifecycleEvidence`; it could not derive a separate Reflection state from the same verified stream. | `reflection_core_and_journal_replay_across_postgres_restart_when_provisioned` uses a new client after a marker-owned PostgreSQL restart and requires `COMPLETED + REFLECTION_FAILED`. | Green after live PostgreSQL restart replay |
| Authorized Hermes history is bounded | The base exposed no typed digest-only history window or cursor and therefore could not bind a candidate to what Hermes was allowed to read. | `authorized_history_pages_are_complete_and_candidate_bound` covers 72 events with stable keyset pages; invalid cursor/limit and stale-page tests reject. | Unit Green |
| Hermes cannot overwrite/delete/core-transition | The base had no Hermes-specific append boundary, so there was no contract preventing an untrusted caller from being handed a broader lifecycle repository. | Separate history/candidate traits expose only typed pages and digest-only candidate append; `retry_generation_preserves_core_and_original_event_commitments` proves the immutable prefix and completed core anchor survive all Reflection tails. | Unit Green |
| Pending queue/claim performs no execution effects | The base had no deterministic known-Task Reflection admission/claim receipt. | `ensure_pending` appends one `EFFECT_INTENT`/outbox admission and `claim_pending` appends one exact evidence receipt; there is no Codex, verification, Git, Hermes, process, path, SQL, or scheduler input in either trait. | Green after live PostgreSQL restart replay |

## Current decisions

- The physical base contains migrations `0001` through `0004`. The first
  slice reuses the verified generic append-only Task Ledger event repository;
  it introduces no global migration or schema-version uplift. This is not a
  global pending scanner or a physically isolated Reflection repository.
- The existing public `ledger_head_digest` keeps its current meaning: the head
  of the whole append-only stream. A new typed `core_head_digest` freezes the
  verified last legal core receipt so Reflection tails cannot rewrite the core
  completion projection.
- `ensure_pending` is an explicit idempotent call after core completion. A
  completed Task with no admission projects the fixed
  `LATTICE_REFLECTION_PENDING_NOT_ADMITTED` condition; status/load never writes
  an admission as a side effect.
- Direct `TASK_FAILURE` and fixed-verifier `OUTPUT_REJECTED` evidence are
  intentionally terminal and queryable, not queueable. Failure learning that
  claims those terminal-core records is a later versioned contract, not an
  implicit capability in this slice.
- TASK-037's preserved Hermes/verifier overlay is explicitly not a dependency
  and is not copied into this worktree.
- The user authorized publishing the original development checkpoint to
  `origin/feature/gh-9-reflection-evolution` at
  `2026-08-11T01:58:40+08:00`. This does not authorize a PR, merge, default-ref
  update, deployment, release, or a claim of final acceptance.
- The user later authorized this GH-9 executor to create one clean feature
  checkpoint commit and non-force push only after focused acceptance passes,
  the worktree is clean except for known GH-9 changes, and remote SHA equality
  is verified.

## Development checkpoint — 2026-08-11T01:58:40+08:00

- Checkpoint base before this commit:
  `2b424ec9a5401a6fbdc4f37d3d401592331afca0`.
- The eight checkpoint paths are limited to Task Domain Reflection types,
  Reflection ports, the runtime Task control projection and tests, and this
  ticket. TASK-037's Hermes/verifier overlay and `docs/modules/**` are excluded.
- Previous checkpoint verification passed `cargo +1.97.1 fmt --all -- --check`,
  all 21 focused runtime Task-control unit tests, and the complete Task
  Domain/Ports test selection with two test threads.
- The marker-owned PostgreSQL restart replay is now required for final local
  completion. It must emit both `GH9_REFLECTION_INITIAL_OK` and
  `GH9_REFLECTION_RESTART_OK` from
  `reflection_core_and_journal_replay_across_postgres_restart_when_provisioned`
  under the existing disposable PostgreSQL harness via
  `scripts/run-task019-postgres.ps1 -RunGh9ReflectionHook` before this ticket
  is considered locally complete.
- Historical note: one earlier reviewed
  preserved-root recovery attempt returned the fixed `REJECTED` result; the
  post-attempt inventory remained 794 entries, one expected junction, the
  original marker digest, no PostgreSQL data/password/PID files, and no process
  image under that root. It was not retried.
- The fresh-lane security gate is satisfied only by the current
  marker-owned disposable PostgreSQL harness. TASK-037 formal verification is
  not part of GH-9 and is not run for this ticket.
- Task Domain 2.3, Ports 1.9, latticed 1.5, SPEC-003 v5, and ADR-024 now record
  the narrow GH-9 Reflection amendment. Therefore local completion is gated on
  current focused verification and publication checks rather than missing
  governance authorization.

## Governance gate

The code changes add Reflection vocabulary to Task Domain and public Reflection
traits to Ports. The authorized finalization amends Task Domain 2.3, Ports 1.9,
latticed 1.5, SPEC-003 v5, and ADR-024 for exactly this known-Task
append-only Reflection lane. The amendment does not authorize MCP expansion,
Hermes execution, unattended claiming, TASK-037 formal run, PR creation, merge,
deployment, or release.
