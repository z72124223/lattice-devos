# ADR-024: Durable Foreman State And Takeover

- Status: accepted for the bounded TASK-079 slice
- Date: 2026-08-21
- Decision owner: user through fixed foreman delegation
- Related: ADR-005, ADR-011, ADR-012, ADR-019, ADR-023, TASK-048, TASK-049, TASK-078

## Decision

Foreman coordination state is represented as a closed, versioned,
secret-free snapshot payload. Task Ledger remains the sole semantic append and
replay owner; Postgres Store remains the sole physical durable owner. A future
`FOREMAN_SNAPSHOT_RECORDED` Ledger event is bound to a fixed foreman-control
stream and guarded by the existing Writer Lease/fencing evidence at every
mutation. No dashboard file, chat/automation history, process list, or in-memory
cache is a status authority.

The snapshot records only bounded worker/thread identity, task, branch/worktree
reference, exact HEAD, state, dependency/blocker reference, latest
heartbeat/report digest, authority/evidence pointers, schema version, and
monotonic generation. It rejects complete chats, prompts, commands,
environments, credentials, tokens, raw provider output, and arbitrary paths.

The snapshot may carry separately typed, expiring digest references to observed
facts, hypotheses, confidence/unknowns, evidence/counterevidence, checked/expiry
time, a refresh trigger, and decision/probe/falsifier records. They are evidence
about a decision, never lifecycle truth: they cannot mark work terminal or
modify state. Learning or promotion is deferred to TASK-084.

Fresh readers reconstruct active/blocked/next-action results only from verified
Ledger replay. The dashboard watchdog is pure and consumes dashboard metadata
only as an untrusted index plus separately supplied live Git/worktree evidence.
It detects stale snapshots, stale/generated mismatch, old HEAD, all missed
heartbeats, and duplicate worker/thread identities. A dependency-blocked result
is retained and never becomes archive-ready through dashboard success alone.

TASK-078's exporter is not modified. Its future integration is a versioned
read-only watchdog input port after TASK-079 proves the payload/replay boundary.

## Consequences

- TASK-079 adds no scheduler, worker/process control, public MCP surface, or
  second writer/truth source.
- PostgreSQL physical migration and live restart acceptance remain gated behind
  focused injectable tests; TASK-051 is untouched.
- Existing TASK-048 observation and TASK-049 closure contracts are read-only
  inputs and do not themselves authorize persistence or archive.

## Rejected alternatives

- Persisting snapshot data in dashboard `status.json`: projection is not truth.
- Persisting automation/chat transcript: privacy and unbounded-content breach.
- Reusing Ledger diagnostic JSON: diagnostics are explicitly non-authoritative.
- A standalone foreman database/table without Ledger append/fencing: second
  truth and writer boundary.
