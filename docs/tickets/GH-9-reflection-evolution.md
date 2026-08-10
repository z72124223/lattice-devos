# GH-9 Reflection Evolution — First Minimal Slice

- GitHub issue: <https://github.com/z72124223/lattice-devos/issues/9>
- Canonical key: `GH-9-reflection-evolution`
- Base: `2b424ec9a5401a6fbdc4f37d3d401592331afca0`
- Frozen implementation branch: `feature/gh-9-reflection-evolution`
- Frozen implementation SHA:
  `a86747ddfa182590c49324a1add15ce4bdc70978`
- Governance branch: `docs/governance-connectivity-first-v2`
- Publication: development checkpoint authorized at
  `2026-08-11T01:58:40+08:00` (`Asia/Taipei`); still `INCOMPLETE`, with no PR,
  merge, deployment, or release authorization

## Goal

Keep the authoritative core Task lifecycle terminal after successful Codex,
verification, Git, and PostgreSQL replay while recording Reflection as an
independent append-only projection. Hermes receives only bounded typed history
and candidate ports; it cannot mutate core transitions or original events.

## Scope

- Separate typed Reflection states: `REFLECTION_PENDING`,
  `REFLECTION_FAILED`, `RETRY_PENDING`, and `DEGRADED`.
- Append-only failure, rejection, Hermes-failure, claim, and candidate events.
- Deterministic known-Task queue, claim, bounded history, and fresh-process
  replay.
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
6. Only a core `COMPLETED` Task enters this first-slice
   queue/claim/candidate/retry lane. `TASK_FAILURE` and fixed-verifier
   `OUTPUT_REJECTED` records on a failed core are terminal, read-only Reflection
   history: they cannot claim, retry, degrade, append a candidate, or revive the
   core Task under this version.
7. Authorized history is a typed keyset page. Its exclusive sequence cursor,
   bounded limit, returned events, next cursor, exact core anchor, and current
   journal head are committed into the page digest; candidate append replays
   the same page before accepting that digest.
8. This slice is a known-Task lane only. It adds no cross-Task discovery,
   `claim_next`, unattended worker, composition hook, MCP writer, or automatic
   production caller. A bounded caller must already hold the exact Task
   binding and invoke the public typed ports explicitly.
9. Runtime-owned pending command identities remain reserved. External mutation
   callers and other event kinds cannot occupy that namespace.

## Workflow ledger

| Step | RED evidence | GREEN evidence | Status |
| --- | --- | --- | --- |
| Core completion survives Hermes failure | `reflection_tail_does_not_rewrite_completed_core_projection` failed because the projection had no independent core anchor: the current stream head moved from `0d6c05…` to `eb2291…`. | A new typed `core_head_digest` is derived from the verified receipt of the last legal core event and stays stable, while the existing public `ledger_head_digest` deliberately remains the current append-only stream head; the exact test passes. | Green |
| Hermes failure is durably persisted | The base had no typed Reflection projector: a generic tail could exist, but no closed `REFLECTION_FAILED` receipt could be replayed. | `hermes_failure_replays_independently_from_completed_core` replays the exact claimed `HERMES_FAILURE` event and leaves the core result/head unchanged. | Unit Green; PostgreSQL restart pending |
| Fresh replay returns core + Reflection | The base fresh-load surface returned only `TaskLifecycleEvidence`; it could not derive a separate Reflection state from the same verified stream. | `reflection_core_and_journal_replay_across_postgres_restart_when_provisioned` uses a new client after a marker-owned PostgreSQL restart and requires `COMPLETED + REFLECTION_FAILED`. | Compiles; live restart pending |
| Authorized Hermes history is bounded | The base exposed no typed digest-only history window or cursor and therefore could not bind a candidate to what Hermes was allowed to read. | `authorized_history_pages_are_complete_and_candidate_bound` covers 72 events with stable keyset pages; invalid cursor/limit and stale-page tests reject. | Unit Green |
| Hermes cannot overwrite/delete/core-transition | The base had no Hermes-specific append boundary, so there was no contract preventing an untrusted caller from being handed a broader lifecycle repository. | Separate history/candidate traits expose only typed pages and digest-only candidate append; `retry_generation_preserves_core_and_original_event_commitments` proves the immutable prefix and completed core anchor survive all Reflection tails. | Unit Green |
| Pending queue/claim performs no execution effects | The base had no deterministic known-Task Reflection admission/claim receipt. | `ensure_pending` appends one `EFFECT_INTENT`/outbox admission and `claim_pending` appends one exact evidence receipt; there is no Codex, verification, Git, Hermes, process, path, SQL, or scheduler input in either trait. | Unit Green; public PostgreSQL replay pending |
| Internal pending namespace cannot be forged | An external caller could reserve a future `gh9-reflection-pending:<generation>` identity and block later progress. | Five public mutation paths reject the internal prefix; replay rejects another event kind using it; focused tests cover no-head-change, forged replay rejection, and normal retry to the next generation. | Unit Green; independent reviewer P0/P1/P2 = 0 |

## Current decisions

- The physical base contains migrations `0001` through `0004`. The first slice
  reuses the verified generic append-only Task Ledger event repository; it
  introduces no global migration or schema-version uplift. This is not a global
  pending scanner or a physically isolated Reflection repository.
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
- The user authorized publishing the frozen implementation checkpoint to
  `origin/feature/gh-9-reflection-evolution` at
  `2026-08-11T01:58:40+08:00`. This does not authorize a PR, merge, default-ref
  update, deployment, release, or a claim of final acceptance.

## Development checkpoint — 2026-08-11T01:58:40+08:00

- Frozen implementation SHA:
  `a86747ddfa182590c49324a1add15ce4bdc70978`.
- Checkpoint base before that commit:
  `2b424ec9a5401a6fbdc4f37d3d401592331afca0`.
- The eight checkpoint paths are limited to Task Domain Reflection types,
  Reflection ports, the runtime Task control projection and tests, and this
  ticket. TASK-037's Hermes/verifier overlay and `docs/modules/**` were excluded
  from the implementation checkpoint.
- Checkpoint verification reported `cargo +1.97.1 fmt --all -- --check`, all 21
  focused runtime Task-control unit tests, and the complete Task Domain/Ports
  test selection with two test threads as passing local evidence. There was no
  remote CI run.
- The marker-owned PostgreSQL restart replay has not passed. Its one reviewed
  preserved-root recovery attempt returned the fixed `REJECTED` result; the
  post-attempt inventory remained 794 entries, one expected junction, the
  original marker digest, no PostgreSQL data/password/PID files, and no process
  image under that root. It was not retried.
- A remaining fresh-lane security gate requires parent, root, and marker
  identity to be derived directly from native creation handles before any live
  replay. No TASK-037 formal or GH-9 live PostgreSQL replay was started for the
  checkpoint publication.
- The frozen implementation SHA is development history only and remains
  `INCOMPLETE_DEVELOPMENT_CHECKPOINT`.

## Governance authorization — 2026-08-11

The responsible user authorized a governance-only branch from the frozen
implementation SHA. No Rust, SQL, Harness, listener, credential, or live
provider effect is authorized by that branch.

The governance branch records:

- Task Domain 2.3: independent Reflection state/failure/candidate vocabulary.
- Ports 1.9: bounded known-Task queue/history/candidate Ports and core-head
  anchoring.
- `latticed` 1.5: real ChatGPT round-trip becomes product P0; GH-9 Reflection
  Ports remain uncomposed.
- ADR-024: governed microkernel plus Port-first/MCP-provider boundaries,
  trustworthy checkpoints, Issue canonicalization, and simplicity-first
  delivery.
- Development Governance v1: Checkpoint Receipt and evidence levels.

This authorizes the documentation and governance meaning of the frozen code. It
does not convert local tests into live acceptance and does not authorize a PR,
merge, deployment, release, GH-9 live replay, or TASK-037 formal.

## Segmented future work

The earlier single GH-9 scope is split to prevent another oversized acceptance
chain:

### GH-9A — Completed-core Reflection projection

- current frozen first-slice semantics;
- governance synchronization;
- native creation-handle containment gate;
- at most one separately authorized live PostgreSQL restart/fresh-process
  replay;
- no real Hermes, global queue, unattended worker, or MCP expansion.

### GH-9B — Failure learning and cross-Task batch discovery

- versioned failed/rejected Task learning contract;
- cross-Task discovery and `claim_next`;
- freshness, claim lease/timeout, and orphan recovery;
- reuse of existing Task Ledger/outbox truth without a second queue authority;
- fake/fixture provider before real Hermes.

### GH-9C — Real unattended Reflection runtime

- low-cost classifier and model routing;
- real Hermes provider;
- unattended scheduling and resource context;
- retention policy;
- independent review, trial, adoption, rollback, and Chinese reports.

Passing GH-9A never implies GH-9B or GH-9C acceptance.

## Current status

```text
FROZEN_IMPLEMENTATION_SHA = a86747ddfa182590c49324a1add15ce4bdc70978
IMPLEMENTATION_STATUS = INCOMPLETE_DEVELOPMENT_CHECKPOINT
GOVERNANCE_STATUS = AUTHORIZED_ON_DOCS_BRANCH_NOT_MERGED
GH9_LIVE_POSTGRES = PAUSED_FOR_ARCHITECTURE_REVIEW
TASK037_FORMAL = PROHIBITED
PR_MERGE_DEPLOY_RELEASE = NOT_AUTHORIZED
CURRENT_PRODUCT_P0 = REAL_CHATGPT_LATTICE_ROUNDTRIP
```