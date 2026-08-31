# ADR-012: Writer Lease Authority, Fencing, And Recovery

- Status: accepted for TASK-014 under the user's 2026-07-29 directive to
  continue the approved LATTICE plan through MVP-3
- Amended: 2026-08-27 for SPEC-011 Phase 4 process-restart recovery
- Date: 2026-07-29
- Decision owner: user
- Related: SPEC-002 v10, SPEC-011, ADR-002, ADR-005, ADR-006, ADR-008,
  ADR-009, ADR-028, TASK-014

## Context

The V1 ProjectLock proves useful one-writer and exact-subject intent, but uses
an unauthenticated local file, a separately writable JavaScript-number
counter, caller time, and validation detached from the eventual durable
mutation. Its fencing counter can be rolled back and reused; it can also issue
an unsafe value beyond `MAX_SAFE_INTEGER` that remains temporarily authorized.

Policy 2.4 likewise accepts lease authority through caller-constructible
booleans, counts, epochs, and fences. Comparing two fields supplied by the same
caller does not prove owner identity or currentness.

## Decision

Create pure Rust `lattice-writer-lease` 1.0. It owns:

- the `VACANT`, `ACTIVE`, and `SUSPECT` aggregate state machine;
- acquire, heartbeat, expiry-based suspect marking, exact release,
  evidence-bound revoke, and reacquire semantics;
- strictly monotonic non-reused fencing allocation;
- exact command idempotency and immutable terminal receipts;
- domain-separated request, transition, authority, command-receipt, and
  aggregate-snapshot hash subjects;
- public pure planning and untrusted aggregate verification used by both the
  deterministic fake and future PostgreSQL adapter.

The TASK-014 fake is process-memory characterization evidence only. It does not
prove concurrency, durability, database time, process death, restart safety,
or live effect authority.

## Identity And Numeric Boundaries

An active/suspect lease binds:

- canonical project ID and immutable Registry snapshot ID;
- task ID, positive Task Spec revision, Task Spec digest, and attempt ID;
- lease ID, holder ID, worktree ID, holder process ID, and process-start
  identity digest;
- daemon instance ID, positive daemon epoch, and positive fencing token.

Daemon epoch, fencing token, and aggregate/lease revisions are represented as
positive signed-BIGINT-compatible values in `1..=i64::MAX`. Allocation checks
overflow before constructing a plan; denial cannot partially advance a
counter, revision, transition, current head, or receipt. A released or revoked
token is never reused.

## State And Command Rules

- `Acquire`: only `VACANT -> ACTIVE`, only in `ACTIVE` admission, with the
  expected vacant head and a newly allocated fence.
- `Heartbeat`: only exact unexpired `ACTIVE -> ACTIVE`, only in `ACTIVE`
  admission; it advances revision and expiry but never changes identity or
  fence.
- `MarkSuspect`: only `ACTIVE -> SUSPECT` when the injected canonical time is
  greater than or equal to expiry. Expiry alone cannot revoke.
- `Release`: only the exact holder/lease/daemon/epoch/fence may move
  `ACTIVE|SUSPECT -> VACANT`; it is allowed in `ACTIVE` or `DRAINING`.
- `Revoke`: only `SUSPECT -> VACANT` with typed exact evidence of either the
  holder process's death or replacement by a strictly newer daemon epoch. It
  is a recovery transition allowed in `DRAINING` or
  `RECONCILIATION_REQUIRED`.
- `Reacquire` is a normal later `Acquire` and receives a strictly newer fence.
- `CANARY` and `STOPPED` admit no user-project Writer Lease transition.
- A suspect lease cannot be revived by heartbeat.

The owner validates injected time, daemon/admission observation, and recovery
evidence. It does not read a clock, inspect a process, advance an epoch, or
change runtime admission.

## Phase 4 Process Handoff Amendment

SPEC-011 restart recovery adds one explicit `ProcessHandoff` transition. It is
not an `Acquire`, an implicit lease steal, or a new worker attempt. It replaces
only the holder OS-process ID and process-start identity after typed
`ProcessDeath` evidence exactly matches the retained holder. Project,
Registry snapshot, task, Task Spec, attempt, lease, holder, worktree, daemon
instance/epoch, acquired time, and fencing token remain unchanged; revision,
heartbeat, expiry, transition chain, and command receipt advance atomically.

`ProcessHandoff` is admitted only while runtime admission is `ACTIVE`, only
against the exact current head, and only within the same daemon
instance/epoch. For an `ACTIVE` lease, observation must strictly follow the
last heartbeat and precede the retained expiry. For a `SUSPECT` lease,
observation must be at or after the retained expiry. The replacement expiry
must be strictly later than both the observation and retained expiry. An exact
retry returns the same receipt; command substitution, leadership evidence,
same process identity, stale head, PID/start mismatch, or counter exhaustion
fails closed.

This amendment preserves the original rule that expiry alone cannot revoke or
steal authority: a handoff always requires independently authenticated death
evidence for the exact PID/start/daemon tuple. If that evidence is unavailable,
the legal path remains exact terminal reconciliation followed by revoke and a
new `Acquire`, which allocates a new fence and belongs to a new attempt.

Owner-verified replay may also locate one historical authority receipt by exact
project and receipt digest, including after release. Only transition-produced
receipts are eligible; zero, duplicate, malformed, truncated, or substituted
history fails closed. This historical lookup proves the old intent binding but
does not make the receipt current or authorize a new effect.

## Idempotency And Replay

Commands are keyed by exact project and command ID. A retry with the same
canonical request returns the identical terminal receipt before stale-head
evaluation. Reusing the command ID with changed content is permanently denied.
Legal denials are terminal and make no lease-state mutation.

Every applied or denied command receipt binds its one-based command
high-water position and the preceding receipt digest. The aggregate separately
claims the command high-water and receipt-chain tail, so removing a denial-only
tail cannot masquerade as unchanged lease state.

Every transition binds the complete before/after authority heads, request
digest, command identity, runtime/admission observation, time observation,
recovery evidence when present, and transition digest. The public verifier
replays raw transition/command rows, rejects unknown versions, reordering,
truncation, duplication, orphan receipts, hash substitution, counter rollback,
and claimed-state disagreement, and returns only a typed verified aggregate.

`verify_snapshot` establishes internal self-consistency. A complete older
history prefix can also be internally self-consistent, so rollback-sensitive
restore additionally calls `verify_snapshot_against_checkpoint` with a
`WriterLeaseCheckpoint` loaded from an independently trusted current row. The
checkpoint binds project, command high-water, command-chain tail, and the
complete snapshot digest. The public validated constructor lets a future store
rebuild that checkpoint without duplicating replay semantics. Deriving the
expected checkpoint from the untrusted snapshot under examination is
forbidden.

## Shared Receipt And Policy Boundary

`lattice-contracts` 1.4 exposes fixed producer
`lattice-writer-lease`, semantic version `1.0`, neutral immutable identity,
status, runtime admission, authority receipt, and full authority head values.
Every security-relevant receipt field is mirrored into the head.
`receipt.head()` is structural projection only.

Policy 2.5 accepts:

- one independently constructed expected writer-use subject;
- one owner receipt;
- an optional complete current head obtained from an independent Writer Lease
  lookup.

Policy removes caller-owned `active`, `current`, `holder_role`,
`current_daemon_epoch`, `current_fencing_token`, and `active_implementers`.
Only the exact Implementer/actor/lease/worktree/daemon/epoch/fence and full
Task Spec binding may pass. For ordinary product-code writer actions, a
missing, suspect, historical, substituted, or head-mismatched receipt denies.
`ReleaseWriter` is the bounded exception defined above: the exact current
holder may release an `ACTIVE` or `SUSPECT` receipt while current admission is
`ACTIVE` or `DRAINING`; every missing, historical, substituted, non-holder, or
head-mismatched release still denies.

Policy keeps no normal dependency on the Writer Lease crate. A test-only
dependency may prove composition using the fake owner's actual current head.

## PostgreSQL Boundary

Step 6 will persist the verified aggregate and call the same planner while
holding the project/counter row lock. PostgreSQL clock, admission, daemon
leadership, lease transition, effect/outbox claim, and every daemon-authorized
durable mutation must be checked within the applicable transaction.

The command row, receipt-chain tail/high-water, complete snapshot digest, and
current checkpoint must advance atomically. Restart and restore must reconstruct
the expected checkpoint from that independently protected current row before
accepting raw history.

The store may map domain values to tables and rows but may not reimplement
state transitions, hashing, idempotency, recovery, or fencing allocation.

## Consequences

- Caller-consistent forged lease facts no longer satisfy Policy.
- Fence rollback, wrap, and reuse become explicit fail-closed conditions.
- Expiry no longer silently steals a live holder's authority.
- Same-attempt OS-process restart is an explicit evidence-bound handoff that
  preserves the existing fence; it cannot be represented as `Acquire`.
- The fake and later PostgreSQL store share one semantic core.
- AC-05 remains open until real multi-connection, DB-clock, restart,
  stale-connection, and same-transaction mutation evidence exists.

## Rejected Alternatives

- Keep V1 filesystem lock/counter as V2 truth.
- Let Policy or `postgres-store` own lease transitions.
- Treat expiry as death.
- Use unconstrained `u64` or JavaScript numbers for PostgreSQL `BIGINT`.
- Trust `receipt.head()` as independent currentness.
- Implement only a fake without a public planner/verifier boundary.
