---
spec_id: SPEC-009
title: Durable foreman runtime checkpoint and restart replay
version: 1
status: approved
approved_by: sole_foreman_delegation
approved_at_local: 2026-08-25
modules:
  - module_id: foreman-state
    constitution_version: 1.3
  - module_id: task-ledger
    constitution_version: 2.7
  - module_id: lattice-ports
    constitution_version: 2.1
  - module_id: orchestrator-runtime
    constitution_version: 2.7
  - module_id: latticed
    constitution_version: 3.1
  - module_id: postgres-writer-lease
    constitution_version: 1.8
  - module_id: postgres-codebase-memory
    constitution_version: 1.3
---

# SPEC-009 — Durable foreman runtime checkpoint and restart replay

## Problem

TASK-079/TASK-094 provide the schema-v6 Ledger event and Writer-owned rebind,
but the product runtime cannot yet write or replay the sole foreman's state.
Closing and reopening Codex therefore loses the verified active/blocked/next
action projection even though PostgreSQL has the intended durable boundary.

## Intended behavior

- The modern canonical MCP surface remains the existing six tools plus exactly
  one `lattice_foreman_checkpoint`; the legacy observer remains exactly two
  delivery tools.
- The checkpoint caller supplies only `checkpoint_id`, exact-next `generation`,
  canonical UTC `occurred_at`, closed state, state-compatible `blocker_ref`, and
  closed heartbeat/evidence SHA-256 pointers. Binding, Git branch/worktree/HEAD,
  Writer authority/fence, database, SQL, path and command remain server-owned.
- The server binds every restart to worker `sole-foreman-v1`, thread
  `lattice-devos-sole-foreman-v1` and the existing fixed foreman Ledger stream.
  Process/session metadata never selects this identity. Its empty history
  accepts only generation 1; every later checkpoint is exactly previous + 1.
- Exact retry is checked from verified durable replay before a new Git or Writer
  observation. A changed payload under one checkpoint ID fails closed.
- Orchestrator orders Writer acquire, fenced Ledger append and known-success
  release. Append-unknown never releases; release-unknown never re-appends.
- Zero-parameter `lattice_runtime_status` includes a verified foreman projection:
  schema, replay/checkpoint status and digests, latest generation, current state
  counts, `NO_DURABLE_SNAPSHOT|CONTINUE|RESOLVE_BLOCKERS|ALL_COMPLETED`, and a
  bounded degraded code. Corrupt, unsupported or unavailable replay is a hard
  tool error; Writer contention may degrade write readiness only after replay.
- `latticed --postgres-initialize` provisions only roles/database/foundation.
  Only the subsequent `latticed --postgres-bootstrap` installs or migrates. It sequences exact
  v5/Writer-v2 through Writer-v3 bridge then Store-v6/rebind, handles v5+bridge,
  exact-v6 retry and v6-current verification, closes migrator credentials,
  creates fresh runtime clients, verifies foreman replay, then may serve MCP.
- Ordinal `0007_foreman_coordination` is the same schema-v6 migration identity,
  with its pre-product Store current-head history guard corrected from six to
  seven exact entries and its SQL/manifest/rebind digests re-pinned. No new
  migration or durable schema is introduced.

## Goals

- Make one sole foreman checkpoint durable and restart-restorable.
- Preserve PostgreSQL/Task Ledger as the only truth and Git as evidence only.
- Preserve product Control scripts and all existing six-tool behavior.

## Non-Goals

- New tables, migrations, dashboards, caches, worker schedulers or generic task
  submission intents.
- Caller-selected identity, authority, filesystem, Git, database or SQL values.
- Migration during normal MCP startup or calls.

## Compatibility, failure and security

Schema-v5 history and all existing MCP responses remain compatible. Unknown
fields, malformed pointers/time and blocker/state mismatch fail before service
dispatch. Generation gaps, changed ID reuse, corrupt replay and partial
bootstrap fail after verified replay but before mutation. PostgreSQL tests use
marker-owned loopback fixtures on a dynamic port excluding 5432 and 58743.

### Versioned wire contract

`lattice_foreman_checkpoint` input is an object with exactly these properties:
`checkpoint_id: string` matching
`^[A-Za-z0-9][A-Za-z0-9._:-]{0,63}$`, `generation: integer > 0`,
`occurred_at: string` (canonical UTC), `state: ACTIVE|BLOCKED|COMPLETED`,
`blocker_ref: string|null` (non-null iff BLOCKED), and `heartbeat_ref` /
`evidence_ref` as lowercase `heartbeat:sha256:<64hex>` and
`evidence:sha256:<64hex>`. Its success object is
`schema: lattice.foreman-checkpoint-result/1.0`, `checkpoint_id: string`,
`generation: integer`, `status: RECORDED|REPLAYED`, `exact_retry: boolean`,
`ledger_digest: lowercase sha256`, and `checkpoint_digest: lowercase sha256`.

`lattice_runtime_status` remains zero-parameter and adds `foreman` with exact
fields: `schema: lattice.foreman-runtime-projection/1.0`,
`replay_status: VERIFIED`, `checkpoint_status: NONE|AVAILABLE`,
`ledger_digest: lowercase sha256`, `checkpoint_digest: lowercase sha256|null`,
`latest_generation: integer`, `active_count`, `blocked_count`,
`completed_count`, `next_action` in the four-value closed enum, and
`degraded_code: null|FOREMAN_WRITER_CONTENTION`. Counts and generation are
non-negative integers.

Closed-schema, format and unknown-field validation fails before service
dispatch as JSON-RPC invalid params (`-32602`) with stable code
`FOREMAN_CHECKPOINT_INVALID`. Changed checkpoint-ID payload is detected only
after verified durable replay and returns a tool result with `isError: true`
and stable code `FOREMAN_CHECKPOINT_ID_REUSE`.
Verified replay failures are hard tool errors:
`FOREMAN_REPLAY_CORRUPT`, `FOREMAN_REPLAY_UNSUPPORTED`, or
`FOREMAN_REPLAY_UNAVAILABLE`. After a valid replay only, Writer contention is
`FOREMAN_WRITER_CONTENTION`; unknown append is
`FOREMAN_APPEND_OUTCOME_UNKNOWN`; unknown release is
`FOREMAN_RELEASE_OUTCOME_UNKNOWN`. No such error carries raw SQL, path, lease,
fence, Git command, database detail or child output, and none is serialized as
a successful structured result.

### Bootstrap and restart matrix

The explicit bootstrap command has these observable rows: `v5 + Writer v2`
applies the fixed Writer-v3 bridge then Store-v6/rebind; `v5 + Writer v3`
applies Store-v6/rebind; `v6 + bridge-pending` retries exact-v6 rebind; and
`v6 + current Writer v3` verifies without mutation. Partial, corrupt,
unsupported and `v6 + Writer absent` profiles fail closed without installing
Writer or changing the Store/Memory/Writer/admission fingerprint. Before any
admission write, Memory- and Writer-owned read-only classifiers independently
verify their exact empty/v2/v3 and v2/v3 catalog, ACL, identity, ledger,
rebind-boundary and replay evidence. Composition accepts only Store-v5 + Memory
`Empty|V2|V3` + Writer fallback, Store-v5 + Memory-v3 + Writer-v3 bridge, or
Store-v6 + Memory-v3 + Writer-v3 pending/current. Exact v6 current additionally requires
the persisted admission authority to equal configuration, performs zero
stop/rebind/restore, then receives full Store/runtime verification from a fresh
Runtime-role Ledger and foreman replay. No-argument startup and every MCP call perform zero
migrations and refuse serving until bootstrap completes. Success closes the
migrator connection, constructs fresh runtime-role clients, verifies foreman
replay through them, and only then reports ready/serves.

For a physically fresh Store, the Writer-owned inspector must first prove the
Writer namespace exactly absent before any Store schema is created; a partial
or corrupt Writer namespace rejects with every Store schema and history still
absent. After that absence proof, bootstrap creates the stopped Store-v5
foundation and reruns the same Memory/Writer closed triple. Store legacy
prefixes are deliberately unsupported by the product bootstrap and reject
before admission load/stop or any Store migration; the Store-owned historical
administrative migration API remains available outside the product entry.

## Acceptance criteria

- [ ] Exact seven modern tools and exact two legacy tools; prohibited checkpoint
      fields reject before service dispatch.
- [ ] Generation `previous + 1`, exact retry, changed ID reuse and gap tests pass.
      Empty replay rejects first generation 2 and distinct process/session
      metadata cannot change the fixed sole-foreman generation chain.
- [ ] Effect-order tests cover acquire, known/unknown append and known/unknown
      release without duplicate append.
- [ ] Explicit bootstrap state matrix and fail-closed partial/corrupt tests pass.
- [ ] Fresh `latticed` process writes a checkpoint; a distinct process replays it
      in zero-parameter Runtime Status without migration.
- [ ] One marker-owned physical database proves: process A runs initialize,
      bootstrap and checkpoint then stops; process B runs no bootstrap and
      returns identical digests/generation/counts/next action with zero
      migration. Corrupt/unsupported/unavailable replay fails hard, and Writer
      contention degrades only after replay validates.
- [ ] Product acceptance runs `npm.cmd run verify`, starts
      `npm.cmd run control:start`, then runs `npm.cmd run control:receipt`; all
      original six MCP tools remain callable and legacy remains exact two.

## Human decisions

None. The current user explicitly authorized this bounded product integration,
non-force feature delivery and later product merge/deploy; this ticket stops
before those parent-owned external gates.
