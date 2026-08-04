# ADR-019: Durable PostgreSQL Task Ledger And Outbox Admission

- Status: accepted under the approved V2 amendment and user's MVP-3 execution directive
- Date: 2026-08-02
- Decision owner: user
- Related: SPEC-002 v23, ADR-005, ADR-008, ADR-011, ADR-018,
  Task Ledger 2.1, Postgres Store 1.3, TASK-021

## Context

TASK-013 established the pure Task Ledger 2.0 event, receipt, replay, and
resource-projection semantics. TASK-020 established a live durable physical
`ControlStore`, but deliberately persisted only opaque commitments. AC-03 and
the durable/restart portion of AC-04 remained open at TASK-020 closure because
no PostgreSQL adapter yet atomically stored the Ledger command, optional event,
head/projection, outbox admission, terminal domain receipt, and physical Store
receipt. TASK-021 implements and verifies that boundary.

Two existing boundaries cannot be bypassed:

1. Task Ledger's event/head/receipt builders and runtime-aware genesis are
   private, while `VerifiedStream` discards verified command records. A live
   adapter would otherwise duplicate domain hashes, retry order, or receipt
   reconstruction.
2. Store v2 persistence evidence currently uses the current database manifest.
   Advancing the global schema without a frozen historical receipt profile
   would make an old exact retry reconstruct a different receipt and falsely
   report retained data as corrupt.

The approved V2 module amendment already directs PostgreSQL to implement
Task Ledger physical persistence without acquiring Ledger event meaning. This
ADR defines that bounded implementation.

## Decision

### Dependency and ownership

Task Ledger 2.1 remains pure and I/O-free. It adds one complete immutable
`verify -> plan -> apply/checkpoint` boundary used by both the fake and live
adapter. Postgres Store 1.3 adds the concrete `PostgresTaskLedger` adapter and
depends one way on Task Ledger:

```text
lattice-postgres-store -> lattice-task-ledger -> lattice-contracts + lattice-cjson + time
```

There is no reverse dependency, adapter-to-adapter call, or new crate. Ports
1.4 remains Contracts-only. Postgres Store owns SQL, locks, connection use,
transaction mechanics, catalog verification, and durability evidence. Task
Ledger remains the sole owner of command, event, receipt, outbox-admission,
projection, replay, and checkpoint meaning.

### Pure Task Ledger 2.1 boundary

Task Ledger 2.1 preserves existing event/request/head/receipt hashes and the
fixed 2.0 shared producer identity. It adds:

- runtime-aware `VerifiedStream::vacant` for structural Fake or Live genesis;
- a single `plan_append` operation that accepts one verified current stream
  and `AppendCommand`, performs exact retry before stale evaluation, and
  returns one indivisible `LedgerAppendPlan`;
- `apply_append_plan`, which rechecks the complete base snapshot commitment;
- verified retained command receipt lookup after restart;
- an immutable `OutboxAdmission` derived only from a successfully appended
  `EFFECT_INTENT` event whose audit outcome is `RECORDED`. Its intent digest is
  the event subject digest. Existing non-`RECORDED` effect-intent combinations
  remain valid appended Ledger events for 2.0 compatibility but derive no
  outbox row; denied or non-effect commands likewise derive none;
- a `LedgerCheckpoint` over complete identity, head, resource projection,
  event sequence/digests, every terminal command request/receipt including
  denials, and every outbox admission in deterministic order;
- untrusted snapshot export plus verification against an independently
  retained checkpoint.

The plan exposes only the typed terminal receipt, exact-retry classification,
new command/event/outbox persistence records, next verified state, record-set
commitment, and next checkpoint. Low-level builders and hash functions remain
private so callers cannot mix artifacts from different base states.

Denied commands do not advance the Ledger event head, but they do advance the
checkpoint. This makes denial-tail truncation or coordinated command
substitution detectable. The fake is refactored to use the same planner.

The frozen new hash subjects are:

- `lattice.task-ledger.outbox-admission` version 1.0 over stream identity/ID,
  event sequence/digest, command/request identity, `RECORDED` intent subject,
  semantic occurrence time, and fixed `ADMITTED` state;
- `lattice.task-ledger.record-set` version 1.0 over the complete new command
  record plus optional new event/outbox record;
- `lattice.task-ledger.checkpoint` version 1.0 over complete identity/runtime,
  current head/resource projection, events in sequence order, commands in
  canonical command-ID order, and outbox admissions in event-sequence order.

Database query/vector order is never a hash input.

### Global schema v3 and frozen Store receipt profile

`0001`, `0002`, and `0003` remain byte-identical. One exact runner-owned
`0004_task_ledger_repository.sql` advances the global schema and reader/writer
compatibility to version 3.

The physical Store contract remains v2. Its receipt persistence profile stays
frozen at physical schema profile 2 and the exact first-three-entry manifest
commitment:

`4582edce68a947998a8f4c6895bb37ceec9e842f516471f4d9e2617a6757f129`

New `store_prepare_v3`, `store_finalize_v3`, and `store_current_head_v3`
functions verify the current global v3 schema/full manifest, but return and
retain the frozen Store v2 profile when constructing or replaying physical
receipts. New Store receipts therefore remain compatible with prior v2
receipts, and an old transaction replays byte-identically after schema
expansion. The three historical `store_*_v2` functions remain present as
immutable catalog history but lose runtime EXECUTE. An old binary cannot write
against global v3.

The verifier reports both current global v3 evidence and the frozen Store
receipt profile. A future migration appends another global profile without
rewriting a historical receipt profile.

### Durable Ledger schema

Schema v3 adds exactly four migrator-owned tables:

1. `control.task_ledger_streams` retains exact stream identity, the complete
   current head/resource projection, counts, and current checkpoint. Every
   authoritative head/projection column is `NOT NULL`. A stream may have
   terminal denials before its first event, so the row retains the complete
   Task-Ledger-derived structural-zero head rather than encoding absence with
   nullable fields or inventing an event.
2. `control.task_ledger_events` retains every authoritative event field under
   unique stream/sequence, stream/command, and event-digest identities.
3. `control.task_ledger_commands` retains the complete canonical request source
   and terminal receipt under unique `(stream_id, command_id)` identity. It
   also binds the base/result Ledger checkpoints and deterministic Store
   transaction ID to the matching physical terminal row; fixed reads return
   the joined historical Store request needed for byte-identical retry.
4. `control.task_ledger_outbox` retains one immutable `ADMITTED` record only
   for a successfully appended `EFFECT_INTENT` with audit outcome `RECORDED`,
   with unique event, stream/command, stream/sequence, admission digest, and
   intent digest.

The Ledger adapter derives its globally unique physical Store transaction ID
as `task-ledger-v1:<sha256>`, where `<sha256>` is the lowercase hexadecimal
SHA-256 of the canonical hash subject
`lattice.postgres-task-ledger.store-transaction-id` version 1.0 over the fixed
Store owner `TASK_LEDGER`, complete stream ID, and complete Ledger command ID.
The 79-byte result satisfies `StoreTransactionId`, is reproducible after
restart or unknown commit outcome, and does not collide with another stream or
repository-owner namespace. The unhashed stream or command identity is never
truncated to fit the Store identifier limit.

The corresponding `StoreScope` and `StoreMutationCommitment` mapping is
exhaustive and frozen:

- `project_id` and `project_snapshot_id` come from the Ledger stream identity;
- `repository_owner` is exactly `TASK_LEDGER`, and
  `aggregate_key_digest` is the complete Ledger `stream_id`;
- `domain_command_digest` is the complete Ledger request digest;
- `record_set_digest` is the plan's Ledger record-set digest;
- `next_state_digest` and `checkpoint_digest` are both the plan's next Ledger
  checkpoint digest;
- `domain_receipt_digest` is the terminal Ledger receipt digest;
- `outbox_intent_digest` is the optional Outbox Admission digest, not the
  event-subject/intent digest. It is absent when no admission exists.

The admitted row separately retains its intent digest, which equals the
`EFFECT_INTENT` event subject digest. This distinction makes the Store bind the
complete admitted record while preserving the original effect identity.

Authoritative scalar fields use explicit SQL columns. The only `jsonb` field
is the bounded, sanitized, non-authoritative diagnostic. SQL rejects JSON
numbers; Rust converts JSON values back to `CanonicalValue`, re-runs
`Diagnostic::new`, and hashes only Rust canonical values, never PostgreSQL
JSON text.

Task Ledger `u64` sequence, resource revision, and counters use constrained
`numeric(20,0)` in PostgreSQL and canonical decimal-text parameters/results.
The accepted range is `0..=18446744073709551615`; this preserves Ledger 2.0
semantics. Store physical revision remains positive signed `BIGINT` and is not
interchanged with domain counters.

### Fixed runtime function surface

Schema v3 owns eleven functions: the retained ungranted three v2 Store
functions plus eight new runtime functions.

Store v3:

1. `control.store_prepare_v3`
2. `control.store_finalize_v3`
3. `control.store_current_head_v3`

Task Ledger v1 persistence surface:

4. `control.task_ledger_prepare_v1`
5. `control.task_ledger_read_head_v1`
6. `control.task_ledger_read_events_v1`
7. `control.task_ledger_read_commands_v1`
8. `control.task_ledger_finalize_v1`

All are fixed-signature, schema-qualified, dynamic-SQL-free,
`SECURITY DEFINER`, non-leakproof, parallel-unsafe, migrator-owned functions
with `search_path = pg_catalog` and row security enabled. Only the eight new
functions grant non-grantable EXECUTE to `lattice_runtime`. `PUBLIC`, Guardian,
reader, and all pre-`SET ROLE` LOGIN identities have none. Runtime has zero
direct table or column SELECT/DML.

`task_ledger_read_events_v1` returns the optional matching outbox fields and
the head surface returns exact event/command/outbox counts. Full Rust replay
and count comparison reject orphan, missing, duplicate, truncated, reordered,
or substituted records.

### Atomic append algorithm

`PostgresTaskLedger` consumes one caller-supplied already-authenticated runtime
client and exact marker-owned target, exposes no client/query/SQL escape, and
uses one bounded `SERIALIZABLE`, synchronous-commit transaction:

1. acquire the fixed stream-scoped lock through
   `task_ledger_prepare_v1`; this read/lock step checks no mutable admission;
2. load the complete fixed-column snapshot and verify it in Rust against the
   independently retained Ledger/physical checkpoint;
3. classify exact stream/command retry or changed command before mutable
   admission. Changed reuse returns no retained receipt;
4. for a new command, run the pure planner and derive the complete Store
   request from the locked current physical head, Ledger record-set digest,
   next checkpoint, domain receipt, and optional outbox admission;
5. call `store_prepare_v3`; new work must match exact ACTIVE daemon
   instance/epoch/authority and the locked physical head. An unexpected
   physical mismatch aborts/retries the whole transaction and never writes a
   stale Store terminal receipt;
6. Rust calls `store_finalize_v3` and then `task_ledger_finalize_v1` inside the
   same transaction. The Ledger finalizer rechecks the base checkpoint and the
   exact matching retained Store terminal/request, inserts the command plus
   optional event/outbox, updates the stream projection/checkpoint, and returns
   only after all fixed row-count invariants hold. Any later failure rolls back
   the preceding Store finalization with the transaction;
7. commit, then return the typed domain receipt, checkpoint, outbox admission,
   global persistence evidence, and Store receipt.

Every new domain command is one applied physical Store mutation, including a
semantic stale/overflow denial whose Ledger event head does not change. The
Store physical state digest equals the complete Ledger checkpoint, so denial
tails and outbox admissions cannot disappear while the retained physical head
remains current.

Exact retry reconstructs the original Store request from the retained domain
command/terminal pair and ignores later admission, epoch, or head changes.
If only one side of the atomic Ledger/Store pair exists, the adapter reports
corruption and never repairs or completes it silently.

The two fixed finalization calls are deliberately sequenced by Rust rather
than nesting Store finalization inside the Ledger SQL function. Expanding the
complete scalar Store plus Ledger record surface into one function exceeds
PostgreSQL's function-argument limit, while accepting table/composite values
would add runtime type/table privilege ambiguity. The selected sequence keeps
both fixed functions, every recheck, and all writes inside one transaction
without granting a generic row or type capability.

### Failure and retry

Only pre-commit serialization/deadlock and the fixed first-row race may retry,
for at most three retries after the initial attempt. An explicit database
response remains a known retryable or terminal result according to its exact
SQLSTATE. Only a commit failure with no database response returns
`CommitOutcomeUnknown`, returns no receipt, and poisons that adapter instance;
a new client plus the exact command is then the only reconciliation path.

Every runtime read/write transaction and each of the eight fixed schema-v3
runtime functions is bounded by `lock_timeout = 5s` and
`statement_timeout = 30s`. Lock/statement timeout SQLSTATEs fail terminally as
`Unavailable`; they never become unknown commit outcomes.

Unavailable, malformed, command substitution, admission denial, authority
mismatch, checkpoint corruption, retained-row corruption, serialization
exhaustion, transaction failure, and unknown commit outcome remain distinct
static errors. Raw SQL, DSNs, credentials, values, and driver diagnostics do
not enter errors, receipts, Debug output, or repository artifacts.

### Upgrade boundary

The runner accepts Fresh, exact v1 prefix, exact v2 prefix, or exact v3 full
state:

- Fresh applies `0002`, `0003`, and `0004`.
- v1 to v3 retains the existing empty physical/terminal precondition before
  `0003` and then applies `0004` in the same runner transaction.
- v2 to v3 may preserve non-empty physical heads/terminal receipts, but only
  after exact v2 history/catalog/ACL/profile verification, exact database
  identity, and `STOPPED`/no-leader admission. Migration locks wait for prior
  runtime transactions to finish.
- v3 exact full state is a verified no-op.

ACTIVE, DRAINING, CANARY, RECONCILIATION_REQUIRED, partial, edited, reordered,
unknown, corrupt, or non-prefix sources fail closed. The migration never
rewrites historical terminal rows.

### Explicit deferrals

TASK-021 admits durable outbox intent but does not claim, deliver, retry, or
reconcile an external effect. Live resource-observation issuance/currentness,
Writer Lease fencing, Registry/Approval/Artifact repositories, Task Domain
projection composition, OpenClaw, Codex, Graphify, Hermes, Codebase Memory,
Guardian activation, production provisioning, release, and deployment remain
later tickets. These deferrals do not prevent AC-03 atomic append/outbox or
AC-04 durable replay/corruption evidence from completing.

## Implementation And Verification Status

TASK-021 completed this accepted decision on 2026-08-02. The final adapter:

- observes dynamic global schema/full-manifest evidence on every read/write
  transaction and compares it with constructor-frozen evidence in that same
  transaction, while retaining immutable Store-v2 receipt evidence;
- accepts only the Store terminal inserted by the current transaction, proven
  by `xmin = pg_current_xact_id()::xid`, before Ledger rows may finalize;
- verifies outbox event, command, and request linkage together;
- rejects duplicate or wrong-project/snapshot physical rows for the complete
  `TASK_LEDGER` stream scope, including a vacant stream with only a Store-side
  orphan; and
- treats the fresh Store genesis separately from the structural vacant Ledger
  checkpoint until the first atomic mutation, after which physical state and
  the complete Ledger checkpoint are identical.

Direct evidence is recorded in the TASK-021 ticket, code/security and
architecture reviews, integration report, workflow ledger, and handoff. The
marker-owned PostgreSQL 17.10 harness passes fresh/v1/v2/v3 migration,
transaction/concurrency/fault/corruption matrices, initial commit, real
restart, and exact replay. AC-03, AC-04, and AC-35 are complete; AC-05, AC-19,
outbox claim/delivery, live resource observation, other durable repositories,
provider integration, production operation, release, and deployment remain
open.

## Consequences

- Task Ledger semantics have one pure implementation shared by fake and live
  persistence; PostgreSQL never reimplements canonical hashes.
- PostgreSQL becomes the durable Ledger/outbox truth without giving runtime
  direct row access or making the domain crate perform I/O.
- Store receipt evidence survives later append-only global migrations.
- Denied commands and outbox admissions become rollback-detectable through the
  same independent checkpoint as event/projection state.
- The schema/function verifier grows substantially and must remain exact.

## Rejected Alternatives

- New adapter crate wrapping `PostgresControlStore`: rejected because the
  public Store owns its client/transaction and cannot atomically add domain
  rows; adapter-to-adapter composition would also violate the current boundary.
- Duplicate Ledger builders in Postgres Store: rejected because hashes, retry
  order, and receipts would gain a second semantic owner.
- Generic JSON/SQL mutation port: rejected because it creates an unbounded
  database command escape hatch.
- Rewrite old Store receipts to the current manifest: rejected because exact
  replay must be byte-identical and historical evidence is immutable.
- Narrow Ledger `u64` to signed `BIGINT`: rejected because it silently changes
  established overflow semantics.
- Execute an outbox worker now: rejected because effect claim/delivery and
  provider reconciliation are separate authority boundaries.
