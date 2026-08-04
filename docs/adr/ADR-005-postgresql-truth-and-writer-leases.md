# ADR-005: PostgreSQL Is the Durable Truth and Lease Authority

- Status: accepted; approved by user on 2026-07-29
- Date: 2026-07-29
- Decision owner: user
- Related: SPEC-002, ADR-001, ADR-002

## Context

ADR-001 selected one hash-chained event truth, but V1 implemented it in
per-task files. ADR-002 selected an exclusive writer lease, but V1 implemented
its durable counter and record in the filesystem.

The V2 product requires PostgreSQL and a durable Codebase Memory. Allowing the
file ledger, file lock, and database to make independent decisions would violate
One Truth.

## Proposed Decision

PostgreSQL becomes the only durable authority for:

- registered projects and repository identities;
- immutable Task Specs and versioned event streams;
- command receipts and idempotency subjects;
- approval nonces and exact approval subjects;
- side-effect intent/outcome outbox records;
- writer fencing counters and active lease projections;
- component capability observations;
- evidence/artifact references;
- memory candidate/review/promotion state;
- release candidate and activation state.

The event store preserves V1 semantics:

- append-only versioned events;
- versioned canonical payload hashing and predecessor hashes;
- `expected_sequence`;
- `command_id` idempotency;
- secret sanitization before persistence;
- deterministic replay and projection verification.

Physical tables, transactions, indexes, and migrations belong to
`postgres-store`. Event meaning/replay belongs to `task-ledger`; project
identity belongs to `project-registry`; lease transitions belong to
`writer-lease`; memory transitions belong to `codebase-memory`. The store
implements their ports but may not invent domain transitions.

## Canonical Bytes And Hashes

V1 and V2 use separate, explicitly versioned algorithms:

- approved V1 fixtures retain the exact historical JavaScript algorithm and
  are read-only compatibility evidence;
- V2 uses `lattice-cjson-1` with UTF-8, Unicode NFC for keys and strings,
  object keys sorted by normalized UTF-8 bytes, minimal JSON escaping, no
  insignificant whitespace, and explicit preservation of `null` versus a
  missing field;
- duplicate object keys after NFC normalization are rejected;
- raw JSON numbers and floating-point values are forbidden in hashed V2
  payloads. Schema-defined integer/decimal values use normalized decimal
  strings; timestamps use schema-defined UTC RFC 3339 strings;
- the hash input is a domain-separation prefix containing schema ID/version and
  canonical-algorithm version followed by the Rust-produced canonical bytes;
- SHA-256 is the initial digest algorithm and its identifier is stored with the
  digest.

PostgreSQL `JSONB` textual output is never hash input. The first contracts
ticket must freeze byte fixtures for key ordering, Unicode normalization,
integer limits, decimals, timestamps, `null`/missing, and V1/V2 separation.
Only a manually approved V1 fixture manifest may define retained compatibility;
known bugs or unverified TASK-004 behavior are not copied automatically.

## Transaction Boundary

An append operation runs as one bounded transaction:

1. sanitize and canonicalize in Rust, then compute the request hash;
2. begin and lock the stream head/serialization row;
3. under that lock, re-read `(stream_id, command_id)`;
4. return its terminal receipt when the request hash matches, or reject a
   different hash;
5. otherwise insert a receipt protected by a unique
   `(stream_id, command_id)` constraint;
6. verify the caller's active daemon epoch/instance, expected sequence, and
   predecessor hash;
7. append events and any effect intent/outbox row;
8. update the stream head and reconstructable projection;
9. finalize the command receipt with the result hash in the same transaction;
10. commit, then wake workers.

Serializable failures retry the whole transaction a bounded number of times.
A duplicate-key race restarts at the receipt read. If the database commit
outcome is unknown, the caller reuses the same command ID and request hash to
query/retry safely. If an external effect outcome is unknown, it becomes
reconciliation work and cannot be reported as success.

Outbox delivery is **at least once**, not exactly once. Every provider effect
must accept a stable idempotency key or expose a status/query reconciliation
contract before it can be enabled.

`LISTEN/NOTIFY` may wake workers but cannot carry authoritative queue state.

## Daemon Leadership And Epoch

- `control.daemon_instances` records a random instance ID, process ID, process
  start identity, binary/manifest digest, start time, heartbeat, and status.
- `control.daemon_leadership` stores the single active instance and a monotonic,
  non-wrapping `BIGINT` epoch.
- The guardian is the only role that may claim/activate/retire daemon
  leadership, and only through narrow, audited stored procedures bound to an
  activation ID.
- Except for guardian-only release/epoch procedures, **every daemon-authorized
  durable mutation** checks the current epoch and instance ID in the same
  transaction. This includes task/event/receipt, project registry, writer
  lease, outbox, artifact metadata, Codebase Memory, review/capability
  observations, and ordinary approval state. Direct table DML is denied to the
  daemon role.
- After an epoch changes, an old daemon is rejected even if its existing
  database connection and credential remain valid.
- Rollback never decrements an epoch. Restarting a prior binary creates a new
  instance at a higher epoch.

## Runtime Admission Gate

`control.runtime_admission` is a guardian-owned projection checked in the same
transaction as every daemon mutation or effect claim:

- `ACTIVE`: the current instance/epoch may perform policy-approved normal work.
- `DRAINING`: new task admission, new writer leases, new outbox/effect claims,
  and new user-project side effects are denied. Only stop, interruption,
  reconciliation/outcome recording, lease release, and drain evidence are
  permitted.
- `CANARY`: the newly activated instance may write only the
  guardian-reserved system health stream through a canary-scoped capability.
  User-project tasks, leases/effects, registry, artifact publication, memory,
  review, capability, and ordinary approval mutations are denied.
- `STOPPED` or `RECONCILIATION_REQUIRED`: every daemon mutation/effect is
  denied.

An artifact provider may create untrusted staging bytes only while holding an
epoch/admission-bound effect claim; publication of metadata/reference always
uses the checked transaction. Stale processes therefore cannot publish durable
artifacts even if they leave disposable staging bytes.

## Writer Lease

- Each project has a `BIGINT` counter and at most one active lease.
- Acquire locks the project/counter row, checks active state, increments only
  after overflow validation, appends evidence, and creates the projection in
  one transaction.
- The lease binds project, task/revision/spec hash, attempt, daemon epoch,
  worktree identity, lease ID, fencing token, and expiry/heartbeat evidence.
- Expiry alone marks a lease suspect. It is not silently broken until holder
  death and reconciliation are evidenced.
- PostgreSQL's clock is the lease time authority. Recovery evidence binds the
  holder's daemon instance ID, process ID, and process-start identity.
- A guardian recovery role or responsible human may request reconciliation.
  Revocation is permitted only when holder death or replaced daemon leadership
  is proven; otherwise the project remains blocked.
- Revoke/reacquire appends evidence and allocates a new fencing token. A prior
  token is never reused.
- Local file/process locks are defense in depth only.
- PostgreSQL advisory locks may reduce contention but are not the durable
  semantic truth.

## Initial Logical Schemas

```text
control:
  projects, repository_identities, event_streams, events, command_receipts,
  approval_nonces, approval_receipts, daemon_instances, daemon_leadership,
  runtime_admission, writer_counters, writer_leases, effect_outbox, artifacts,
  component_capabilities, release_candidates, release_events,
  activation_projection

memory:
  code_snapshots, sources, records, record_sources,
  retrieval_runs, search_documents

readmodel:
  rebuildable task, agent, and upgrade projections
```

Exact SQL, indexes, retention, and role grants belong to approved tickets and
migrations, not this ADR.

## Memory Boundary

PostgreSQL full-text search is the zero-extension baseline to benchmark, not a
claim that it is sufficient. The first memory ticket measures recall,
precision, and correct no-answer behavior for Traditional Chinese, mixed
Chinese/English, Rust symbols/paths, error codes, and exact filenames. A
trigram strategy, additional token table, or approved extension is considered
only if the baseline misses the accepted threshold. Memory remains
informational: it cannot authorize policy, scope, approvals, leases, or
releases.

## Compatibility And Migration

- This ADR supersedes only the V1 file-storage implementation portions of
  ADR-001 and ADR-002; their single-truth, approval, and writer invariants
  remain.
- V1 files remain immutable historical evidence.
- A future importer must verify old chains read-only, dry-run, import once, and
  compare heads/projections. This repository does not claim such data exists.
- The first A/B activation MVP permits **no schema migration**. Active A,
  shadow B, candidate B, and rollback A must all use the already-active schema.
- A later expansion-only protocol must separately prove both A and B compatible,
  acquire a migration lock, append intent/outcome events, recover after
  interruption, and preserve rollback. Destructive contract migrations require
  separate approval, backup/restore evidence, and never run as automatic A/B
  activation.

## Official Contract Evidence

- [PostgreSQL transaction isolation](https://www.postgresql.org/docs/current/transaction-iso.html)
- [PostgreSQL advisory locks](https://www.postgresql.org/docs/current/explicit-locking.html#ADVISORY-LOCKS)
- [PostgreSQL JSON types](https://www.postgresql.org/docs/current/datatype-json.html)
- [PostgreSQL full-text search](https://www.postgresql.org/docs/current/textsearch.html)
- [PostgreSQL NOTIFY transaction behavior](https://www.postgresql.org/docs/current/sql-notify.html)

## Consequences

- One database transaction can bind intent, event, receipt, projection, and
  outbox state.
- PostgreSQL availability and migration quality become platform-critical.
- A running service is not enough; least-privilege roles, connection,
  backups/recovery, migration checks, and concurrency tests remain future
  evidence gates.

## Approval Gate

Accepting this ADR authorizes schema/constitution/ticket design. It does not
authorize creating a database, role, credential, extension, or migration.
