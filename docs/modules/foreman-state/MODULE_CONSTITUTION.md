---
module_id: foreman-state
name: Foreman State
version: 1.4
status: active
owner: LATTICE maintainers
last_reviewed: 2026-08-25
---

## Mission

Define the small, versioned, secret-free foreman snapshot and pure watchdog
classification used to reconstruct active engineering work after a fresh
process. It gives Task Ledger one typed semantic payload; it never becomes a
second durable control-plane truth.

## Non-Goals

- Persist rows, choose a database, acquire a Writer Lease, control workers, or
  archive a Codex task.
- Store a chat transcript, prompt, command line, environment, credential,
  secret, raw stderr, or arbitrary path.
- Treat `status.json`, a dashboard, a process list, or a heartbeat as authority.

## Owned Data

- Snapshot schema/version, bounded worker/thread/task identity references,
  state, dependency/blocker, latest heartbeat/report digest, authority/evidence
  references, and generation ordering.
- Versioned `lattice.foreman-epistemic/1.0` expiring-reference digests: observed facts, hypotheses, unknowns,
  evidence/counterevidence, decision/probe/falsifier, confidence, checked/expiry
  time, and a closed refresh trigger. These are never snapshot state.
- Pure reconstruction and watchdog classifications over already loaded typed
  snapshots plus independently supplied live Git/worktree observations.

Task Ledger owns append order, hash-chain replay, idempotency, and authoritative
current projection. Postgres Store owns physical rows and transactions. The
dashboard remains an untrusted read-only projection.

## Public Contracts

- Construct only schema `lattice.foreman-snapshot/1.0` from bounded typed
  fields and reject secret-like or transcript-bearing inputs.
- Accept hypotheses only as expiring digest pointers. Their text, promotion,
  learning, and authority remain outside this module and are deferred to TASK-084.
- Derive active, blocked, archive-ineligible, stale, duplicate-identity, and
  next-action projections without I/O.
- Compare dashboard metadata with live Git/worktree observations and report
  drift; it cannot repair, archive, write, or grant authority.
- Validate the closed caller-owned checkpoint fields and reconstruct the
  bounded Runtime Status counts/next action from replay-verified snapshots.
- Validate and canonically encode one bounded dependency blocker, then
  reconstruct its blocked/resumed next action from verified snapshot history
  without performing Git or filesystem I/O. Promote the scalar only when its
  domain-separated evidence commitment matches, so canonical-looking legacy
  strings remain opaque blockers.

## Invariants

1. Only Task Ledger's verified replay can make a snapshot authoritative.
2. Snapshot generation is positive and exactly the previous generation plus
   one per foreman/worker identity; an old HEAD, gap, or stale report never
   becomes healthy by omission.
3. A dependency-blocked worker is never archive-ready solely from a terminal
   dashboard outcome.
4. Duplicate worker/thread identities, malformed evidence pointers, unknown
   schema/state, stale freshness, and secret/transcript content fail closed.
5. The watchdog has no filesystem, Git, process, database, network, dashboard,
   MCP, or scheduler dependency.
6. Epistemic references are opaque, bounded, separately typed inputs; no
   hypothesis, confidence, or expired record can serialize as authoritative
   lifecycle state or change it without a later Ledger-authorized decision.

## Allowed Dependencies

- Rust standard library.
- `lattice-contracts` immutable worker-observation identifiers only.
- `lattice-cjson` canonical digest mechanics only.

## Forbidden Dependencies

- Task Ledger, Ports, Orchestrator, Postgres Store, Writer Lease, dashboard
  exporter, concrete Git/process clients, provider SDKs, credentials, or product
  repositories.

## Failure, Compatibility, And Migration

Unknown schema versions and malformed/oversize/secret-bearing records reject
without producing an active or archive-ready projection. A later schema requires
a new versioned payload plus Task Ledger/Postgres compatibility evidence; old
snapshots remain replayable and are never silently rewritten.

## Acceptance Gates

| Gate | Evidence | Owner | Required for merge |
|---|---|---|---|
| Pure schema/replay | focused snapshot and fresh-reader tests | Engineering | yes |
| Privacy rejection | transcript/secret/path rejection matrix | Security review | yes |
| Watchdog drift | stale HEAD, all-missed-heartbeat, duplicate identity, blocked archive tests | Engineering | yes |
| Ledger/Postgres binding | append/replay and injectable repository conformance | Architecture review | yes |

## Change Policy

Schema fields, generation ordering, snapshot state meaning, privacy limits, or
dependency direction require a versioned amendment, Task Ledger/Postgres review,
and responsible-user authorization.

## Amendment History

| Version | Date | Decision reference | Summary | Approver |
|---|---|---|---|---|
| 1.1 | 2026-08-21 | ADR-024, SPEC-006, TASK-079 | Add separately typed, expiring epistemic references without lifecycle authority | Foreman-delegated user authority |
| 1.2 | 2026-08-21 | ADR-024, SPEC-006 v3, TASK-079 | Export fixed-scalar snapshot reconstruction values for the Ledger-owned typed persistence boundary; no I/O or authority added | Fixed-foreman delegation |
| 1.3 | 2026-08-25 | ADR-027, SPEC-009, TASK-105 | Add closed checkpoint intent/status projection and require exact-next generation; server observation and I/O remain outside | Sole-foreman delegation |
| 1.4 | 2026-08-25 | SPEC-010, TASK-106 | Add one closed dependency blocker and pure blocked/resumed replay projection; Git ownership and integration proof remain server-owned I/O | Explicit user delegation |
| 1.0 | 2026-08-21 | ADR-024, SPEC-006, TASK-079 | Initial secret-free snapshot and read-only watchdog boundary | Foreman-delegated user authority |
