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
    constitution_version: 2.7
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
- Exact retry is checked from verified durable replay before a new Git or Writer
  observation. A changed payload under one checkpoint ID fails closed.
- Orchestrator orders Writer acquire, fenced Ledger append and known-success
  release. Append-unknown never releases; release-unknown never re-appends.
- Zero-parameter `lattice_runtime_status` includes a verified foreman projection:
  schema, replay/checkpoint status and digests, latest generation, current state
  counts, `NO_DURABLE_SNAPSHOT|CONTINUE|RESOLVE_BLOCKERS|ALL_COMPLETED`, and a
  bounded degraded code. Corrupt, unsupported or unavailable replay is a hard
  tool error; Writer contention may degrade write readiness only after replay.
- Only `latticed --postgres-bootstrap` installs or migrates. It sequences exact
  v5/Writer-v2 through Writer-v3 bridge then Store-v6/rebind, handles v5+bridge,
  exact-v6 retry and v6-current verification, closes migrator credentials,
  creates fresh runtime clients, verifies foreman replay, then may serve MCP.

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
fields, generation gaps, changed ID reuse, malformed pointers, non-canonical
time, blocker/state mismatch, corrupt replay and partial bootstrap fail before
dispatch or mutation. PostgreSQL tests use marker-owned loopback fixtures on a
dynamic port excluding 5432 and 58743.

## Acceptance criteria

- [ ] Exact seven modern tools and exact two legacy tools; prohibited checkpoint
      fields reject before service dispatch.
- [ ] Generation `previous + 1`, exact retry, changed ID reuse and gap tests pass.
- [ ] Effect-order tests cover acquire, known/unknown append and known/unknown
      release without duplicate append.
- [ ] Explicit bootstrap state matrix and fail-closed partial/corrupt tests pass.
- [ ] Fresh `latticed` process writes a checkpoint; a distinct process replays it
      in zero-parameter Runtime Status without migration.
- [ ] Product Control scripts and existing six tools regress green.

## Human decisions

None. The current user explicitly authorized this bounded product integration,
non-force feature delivery and later product merge/deploy; this ticket stops
before those parent-owned external gates.

