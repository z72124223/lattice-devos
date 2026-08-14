# ADR-020: Durable PostgreSQL Global Project Registry

- Status: accepted after independent TASK-022 governance re-review under the
  approved V2 amendment and user's MVP-3 execution directive
- Date: 2026-08-03
- Decision owner: user
- Related: SPEC-002 v32, ADR-005, ADR-008, ADR-010, ADR-016, ADR-017,
  ADR-019, Project Registry 1.2, Postgres Store 1.7, TASK-022, TASK-075

## Context

Project Registry 1.1 already owns the deterministic global lifecycle for all
registered projects. Its one in-memory owner contains every project and every
command, and accepted plus pending identity collision checks scan across that
whole state. A registration can terminate as `Denied` without creating any
project authority or `ProjectSnapshotId`; an authoritative observation can
instead terminate as state-changing `Blocked` while preserving another
project's accepted or pending reservation.

ADR-016 created a project-scoped physical `StoreScope` containing an exact
`ProjectId` and immutable `ProjectSnapshotId`. That is truthful for aggregates
already bound to one project snapshot, but not for the global Registry:

- registration denial has no authority snapshot to put in the scope;
- a before snapshot cannot represent registration and an after snapshot cannot
  represent denial;
- using the colliding project's snapshot would be cross-project substitution;
- a sentinel or fabricated snapshot would falsely label persistence evidence;
- separate per-project physical heads cannot serialize accepted/pending
  collisions into one deterministic Registry-owned command history;
- a legal Registry authority snapshot can exceed StoreScope's separate
  128-byte identifier bound because it contains a project ID, revision, and
  observation digest.

Therefore TASK-022 must not weaken `StoreScope`, change Store-v2 receipt hashes,
or move `Denied`/`Blocked` meaning into a SQL unique constraint. The approved
V2 amendment already plans one separately governed domain repository at a
time. This ADR defines the Registry-specific global exception.

## Decision

### Registry remains the semantic owner

Project Registry 1.2 remains pure Rust and I/O-free. It adds one public
runtime-aware global verified-state boundary used by both Fake and PostgreSQL:

- a vacant `Fake` or `Live` Registry state;
- an immutable `RegistryCheckpoint`;
- one `plan_command` operation over a verified current state and typed
  `RegistryCommand`;
- one `apply_command_plan` operation that rechecks the complete base
  checkpoint;
- complete verified command records containing the original typed request,
  unchanged semantic receipt, non-zero global ordinal, and base/result
  checkpoints;
- export of untrusted observation, project, command, and reservation
  projections;
- reconstruction by replaying commands in ordinal order from a vacant state,
  followed by exact comparison with every retained projection and independently
  stored checkpoint.

`RegistryCheckpoint::from_retained` reconstructs an independently read
checkpoint value without claiming it is current. Plain
`verify_untrusted_registry_snapshot` proves only internal self-consistency.
`verify_untrusted_registry_snapshot_against_checkpoint` additionally requires
the separately read retained singleton checkpoint and is the only verifier a
durable adapter may use before returning current authority. A self-consistent
older prefix therefore cannot hide a removed denial or exact-observation tail.

The existing observation, command-request, authority-receipt, and
command-result hash subjects remain unchanged. The existing `result_digest` is
the terminal semantic command-result commitment; Registry 1.1 has no separate
terminal-receipt or record-set hash subject. Checkpoint and record-set subjects
are new in Registry 1.2.
Authority construction becomes runtime-aware, but Fake execution still forces
`RuntimeKind::Fake` and must reproduce every TASK-012 receipt byte-for-byte.
The PostgreSQL adapter may return a planned Live semantic receipt only after
the matching transaction commits.

Before extracting the shared planner, representative Registry 1.1 Fake
observation, request, authority-receipt, and command-result digests are frozen
as literal golden vectors. The new 1.2 vacant checkpoint, non-vacant
checkpoint, and record-set subjects receive their own literal vectors when
first introduced. Planner extraction is accepted only when the 1.1 vectors and
every existing TASK-012 behavior remain byte-identical.

### Global checkpoint and command order

The Registry checkpoint uses a new domain-separated canonical subject
`lattice.project-registry.checkpoint` hash-domain version `1`. A checkpoint high-water is
zero only for a vacant Registry; first-seen command records have the strict
positive sequence `1..N`. It binds:

- Registry runtime and checkpoint version;
- a non-negative, signed-BIGINT-compatible, non-wrapping global command
  high-water;
- complete current project projections in canonical project-ID order;
- accepted and pending identity reservations in canonical dimension/digest
  order;
- complete first-seen command request and semantic receipt records in ordinal
  order;
- project, command, reservation, and retained-byte counts.

Its canonical object has exactly `schema_version`, `runtime`,
`command_ordinal`, `observation_count`, `project_count`, `command_count`,
`reservation_count`, `retained_bytes`, and `logical_state`. Ordinals, counts,
and bytes are canonical decimal strings; `logical_state` is the complete
object defined below.

### Acyclic canonical commitment graph

Registry 1.2 freezes three distinct canonical projections and this construction
order:

1. The checkpoint command core contains only ordinal, the complete typed
   request, and the complete semantic `RegistryCommandReceipt`. It excludes
   base/result checkpoint references, `record_set_digest`, retained-byte/count
   fields, transaction/persistence digests, database identity, and schema
   evidence.
2. From the planned state, build the domain logical-retained-state projection
   and its byte count, then build the result checkpoint. Command checkpoint
   references are verified as a strict chain but are not checkpoint inputs.
3. `lattice.project-registry.record-set` hash-domain version `1` binds the command
   persistence core (checkpoint command core plus base/result checkpoint
   references), any newly inserted immutable observation, an optional current
   project replacement, and the exact ordered reservation deletes/inserts. It
   excludes its own digest and every PostgreSQL/adapter field.
4. Only after the result checkpoint and record-set exist does the adapter build
   the Registry transaction digest, followed last by
   `lattice.postgres-project-registry.receipt` hash-domain version `1`. Adapter evidence is
   never a Project Registry checkpoint, logical-byte, or record-set input.

This order is normative. No physical command-row convenience projection may be
substituted for one of these domain projections.

Every first-seen terminal command advances the global ordinal/checkpoint once,
including zero-mutation `Denied`, state-changing `Blocked`, and an exact
no-project-change `Observe`. Only legal lifecycle mutations advance the target
project's Registry revision and authority snapshot. Same command plus same
request returns its identical historical semantic receipt and checkpoints
without advancing global state. Same command plus changed request is rejected
without receipt disclosure or mutation.

### Bounded retained state

Project Registry 1.2 fails closed before mutation when its retained-state
limits would be exceeded:

- at most 4,096 current project projections;
- at most 65,536 first-seen terminal command records;
- at most 67,108,864 bytes in the Registry-owned retained snapshot accounting;
- at most 131,072 UTF-8 bytes for one already-NFC canonical-root observation.

Exact replay and changed-ID classification precede capacity checks. These are
MVP-1 safety bounds, not a deletion or compaction policy. A future versioned
snapshot/archive design is required before increasing or compacting them.

`lattice.project-registry.logical-retained-state` schema version `1` defines the exact
67,108,864-byte accounting subject. Its `lattice-cjson-1` object contains, in
this schema order before canonical key sorting, `schema_version`, `runtime`,
`observations`, `projects`, `commands`, and `reservations`:

- observations are complete and digest-keyed, sorted by digest, and counted
  once even when several commands/projects reference them;
- projects are current projections sorted by Project ID and reference accepted
  or pending observations by digest;
- commands are checkpoint command cores sorted by strict ordinal;
- reservations are complete accepted/pending projections sorted by identity
  dimension, identity digest, status, and Project ID;
- optional fields are explicit canonical `null`, text is NFC and measured as
  encoded UTF-8, and unsigned/count values are canonical decimal strings.

The retained count is exactly the length of `canonicalize(logical_state)` and
does not include a hash frame, base/result checkpoint references, counts, the
retained-byte field itself, checkpoint or record-set digests, SQL row overhead,
database/schema evidence, transaction fields, or persistence receipts. The
checkpoint contains the logical state, its independently recomputed counts,
and this byte count, so SQL may compare the value but never calculate domain
accounting.

The vacant logical-state canonical bytes are exactly:

```text
{"commands":[],"observations":[],"projects":[],"reservations":[],"runtime":"LIVE","schema_version":"1"}
```

They are 103 bytes. With zero counts and high-water `0`, the frozen vacant
checkpoint digests are `22ad9599c05ab384e720b8f1d91bdfbe1262360850602aa0f6b5fd79c1797f4f`
for Fake and `5bb1f9d9adf7228bef7ea45cc029e79d71761c4dfed3abe086634917db38c173`
for Live.

### Registry-specific durable evidence

Postgres Store 1.4 adds `PostgresProjectRegistry`; it depends one way on the
Project Registry planner/verifier. It does not call `PostgresControlStore`,
construct a `StoreScope`, or return a `StoreTransactionReceipt`.

For each first-seen command it retains one immutable Registry persistence
receipt under `lattice.postgres-project-registry.transaction` and
`lattice.postgres-project-registry.receipt`, both with hash-domain version `1`.
The receipt binds:

- fixed adapter producer/version, Live/DurablePostgres meaning, and exact
  Registry catalog identity;
- target project ID, command ID, semantic request digest, semantic result
  digest, and record-set digest;
- complete base and result Registry checkpoints and global ordinals;
- original complete daemon instance/epoch/admission/revision/observation/head
  authority used for the durable mutation;
- database identity commitment and current global schema version/manifest;
- transaction digest and persistence receipt digest.

This evidence proves only that the Registry-owned plan committed in the exact
database/checkpoint history. It is not project authority, a Store receipt,
filesystem/Git inspection evidence, merge readiness, or protected-action
approval. Exact retry returns the original persistence receipt even after the
Registry checkpoint, daemon epoch, or admission changes.

Contracts 1.9 and Ports 1.4 do not change. If a future design wants one generic
receipt for both project-scoped and global aggregates, it must add a typed
global Store scope and version the shared Store contract; it may not overload
`ProjectSnapshotId`.

### Schema v4

Migrations `0001` through `0004` remain byte-identical. Exact
`0005_project_registry_repository.sql` advances the global manifest and
reader/writer compatibility to schema v4 and adds exactly five authoritative
tables:

1. `control.project_registry_state` is one singleton global serialization row
   containing runtime, counts, retained-byte accounting, current ordinal, and
   complete checkpoint.
2. `control.project_registry_observations` contains each immutable complete
   observation keyed by its semantic digest, including canonical root and all
   root/repository/file/primary-ref identity fields required to reconstruct an
   original command without duplicating Registry legality in SQL.
3. `control.project_registry_projects` contains each current accepted
   observation, optional pending observation, drift projection, project class,
   lifecycle/revision, and current authority receipt projection.
4. `control.project_registry_commands` contains the complete ordered typed
   command request, including references to immutable observations; the fixed
   terminal outcome/denial/drift and before/after/authority digest projection
   from which pure replay reconstructs and compares the complete semantic
   receipt; the checkpoint chain, record-set, original daemon authority,
   database/schema commitments, and immutable Registry persistence receipt.
5. `control.project_registry_identity_reservations` normalizes accepted and
   pending root/repository/file identity digests under one global uniqueness
   key. SQL uniqueness is corruption defense only; pure Registry replay alone
   decides `Denied`, `Blocked`, and reservation ownership.

All authoritative values use fixed columns. No Registry command, observation,
authority, denial, drift, reservation, checkpoint, or receipt is stored as
`jsonb`, an arbitrary map, an opaque canonical blob, or caller-defined SQL.
Domain `u64` Registry revisions use constrained `numeric(20,0)` and canonical
decimal text. The singleton high-water is non-negative signed `BIGINT`; command
row ordinals are positive signed `BIGINT` in the strict sequence `1..N`.

### Forward-profile-bound runtime surface

Schema v4 introduces successor functions whose parameters bind the caller's
constructor-frozen current global schema/manifest. They compare that evidence
with the transaction's live compatibility row and recomputed migration history.
An older binary therefore fails closed after a later schema expansion, while a
future compatible binary may use the same semantic function body with a new
explicit profile.

The v4 runtime allowlist is:

- three `store_*_v4` functions preserving Store-v2 receipt meaning;
- five `task_ledger_*_v2` functions preserving Task Ledger 2.1 rows,
  checkpoints, and semantic receipts;
- nine `project_registry_*_v1` functions for prepare; reads of state,
  observations, projects, commands, and reservations; staging a command plus
  any immutable observation; staging a project plus reservations; and final
  checkpoint publication.

Schema v4 has exactly 15 `control` tables, 28 retained catalog functions, and
17 runtime-executable functions. It adds five tables and 17 functions to the
v3 profile: three Store-v4, five Task-Ledger-v2, and nine Registry-v1. All 11
historical functions remain present and have no runtime EXECUTE.

### Exact Registry function signatures and parameter budget

All inputs are scalar. `P` is the ordered current-profile pair
`(smallint global_schema_version, text global_manifest_sha256)`. `C` is the
ordered checkpoint tuple `(text runtime, bigint ordinal, bigint
observation_count, bigint project_count, bigint command_count, bigint
reservation_count, bigint retained_bytes, bytea digest)`. `H` is
`(boolean present, text producer_id, text producer_version, text runtime, text
project_id, text snapshot_id, numeric registry_revision, text lifecycle, text
project_class, text primary_ref, bytea primary_ref_storage_digest, bytea
observation_digest, bytea receipt_digest)`. `A` is the daemon-authority tuple
`(text runtime, text daemon_instance_id, bigint daemon_epoch, text admission,
bigint revision, bytea observation_digest, bytea head_digest)`.

The exact ordered input manifest is:

| Function | Ordered scalar inputs | Count |
|---|---|---:|
| `project_registry_prepare_v1` | `P`; command `(text command_id, bytea request_digest)`; `A`; `bytea expected_base_checkpoint_digest` | 12 |
| `project_registry_read_state_v1` | `P` | 2 |
| `project_registry_read_observations_v1` | `P` | 2 |
| `project_registry_read_projects_v1` | `P` | 2 |
| `project_registry_read_commands_v1` | `P` | 2 |
| `project_registry_read_reservations_v1` | `P` | 2 |
| `project_registry_stage_command_v1` | the 73-input expansion below | 73 |
| `project_registry_stage_project_v1` | the 22-input expansion below | 22 |
| `project_registry_finalize_v1` | the 27-input expansion below | 27 |

The 73 stage-command inputs are, in order: `P`; command `(bigint ordinal, text
command_id, text action, text project_id, text project_class, bytea
observation_digest)`; `H`; request tail `(text decision, bytea evidence_digest,
bytea request_digest)`; terminal projection `(text outcome, text denial_reason,
text denial_dimension, text denial_existing_project_id, text denial_lifecycle,
text denial_expected_decision, text denial_found_decision, bytea
before_receipt_digest, bytea after_receipt_digest, bytea
authority_receipt_digest, boolean drift_canonical_root, boolean
drift_repository, boolean drift_file, boolean drift_primary_ref_name, boolean
drift_primary_ref_storage, bytea result_digest)`; base `C`; result `C`; `bytea
record_set_digest`; `A`; `(bytea transaction_digest, bytea
persistence_receipt_digest)`; and optional-observation staging `(boolean
stage_observation, text canonical_root, bytea root_identity_digest, bytea
repository_identity_digest, bytea file_identity_digest, text primary_ref,
bytea primary_ref_storage_digest)`. Nullable variant fields remain scalar SQL
NULLs; no omitted positional field or alternate overload exists.

The 22 stage-project inputs are, in order: `P`; `(text project_id, text
project_class, bytea accepted_observation_digest, bytea
pending_observation_digest)`; five ordered drift booleans; and authority
`(smallint contract_version, text producer_id, text producer_version, text
runtime, text snapshot_id, numeric registry_revision, text lifecycle, text
primary_ref, bytea primary_ref_storage_digest, bytea observation_digest, bytea
receipt_digest)`. The function derives the exact accepted/pending reservation
rows only from the already staged immutable observations and replaces only that
project's reservations.

The 27 finalizer inputs are, in order: `P`; `(text command_id, bigint
ordinal)`; base `C`; result `C`; `(bytea record_set_digest, bytea
transaction_digest, bytea persistence_receipt_digest, boolean
stage_observation, boolean stage_project, bigint reservation_delete_count,
bigint reservation_insert_count)`. Static catalog tests assert these exact
identities and PostgreSQL's 100-input limit; the maximum is 73.

All prior Store-v3 and Task-Ledger-v1 functions remain immutable catalog
history but lose runtime EXECUTE. Every new function is fixed-signature,
schema-qualified, dynamic-SQL-free, migrator-owned `SECURITY DEFINER`,
non-leakproof, parallel-unsafe, row-security-on, and configured with
`lock_timeout = 5s` and `statement_timeout = 30s`. Runtime has zero direct
SELECT/DML on all protected tables.

### Transaction order and partial-state defense

`PostgresProjectRegistry::execute` uses one caller-supplied authenticated
client and one bounded `SERIALIZABLE` transaction. Besides the fixed 5-second
lock and 30-second statement timeouts, Rust sets
`idle_in_transaction_session_timeout = 30s` locally and enforces a 45-second
monotonic begin-to-pre-commit deadline, checked after every read batch, after
pure replay, and before staging, finalization, and commit. An idle timeout or
pre-commit deadline rolls back and returns typed `Unavailable`; it is never a
commit-unknown result and is not automatically retried:

1. compute the exact semantic request digest;
2. call fixed prepare, which classifies exact/changed command reuse before
   mutable checks; exact replay bypasses current admission;
3. for new work, verify exact ACTIVE daemon authority/admission/global profile
   and lock the singleton Registry state row;
4. read observations, projects, commands, and reservations in that same
   transaction;
5. reconstruct and verify the pure Registry state/checkpoint;
6. plan one command in Project Registry 1.2;
7. stage the complete command plus any new immutable observation and, only
   when changed, the target project plus its reservations through fixed
   functions;
8. finalization accepts only rows whose `xmin` belongs to the current
   transaction, rechecks base/result checkpoint, record-set, counts, and stage
   shape, then publishes the singleton result checkpoint;
9. commit once and only then return semantic plus persistence receipts.

The split fixed calls avoid PostgreSQL's 100-argument function limit without
using composite/table arguments, builtin arrays as row maps, JSON payloads, or
extra runtime type privileges. A failure after any stage rolls back all rows.
If a caller deliberately commits a stage without finalization, the unchanged
singleton checkpoint disagrees with retained rows; every later load fails
closed and returns no Registry authority. Such a partial is never silently
completed, repaired, deleted, or interpreted as a terminal domain command.

### Migration and compatibility

The runner accepts only Fresh, exact v1, exact v2, exact v3, or exact v4. The
v3-to-v4 path may contain Store and Task Ledger history only after the complete
v3 catalog, manifest, ACL, function, and receipt profiles pass and admission is
`STOPPED` with no leader. It seeds exactly one Live vacant Registry singleton
with high-water/counts zero, retained bytes 103, and the frozen Live vacant
checkpoint digest above; the other four Registry tables and command history
remain empty. This row is the lock target for the first command. The migration
never rewrites an old physical or Ledger terminal row.

Schema v4 must prove:

- historical Store-v2 receipts replay byte-identically;
- historical and new Task Ledger commands/checkpoints replay identically and
  new Ledger appends remain available through the v2 fixed-function surface;
- fresh/v1/v2/v3/v4 migration, rollback, concurrent runner, ACTIVE denial,
  dynamic manifest drift, ACL, and restart behavior;
- Registry command/project/reservation/checkpoint/persistence corruption and
  current-transaction partial stages fail closed.

### Failure and reconciliation

Explicit PostgreSQL responses remain known retryable or terminal outcomes.
Serialization/deadlock conflicts receive at most three pre-commit retries.
Lock timeout `55P03` and statement timeout `57014` are bounded unavailable
outcomes. Only commit failure with no database response is outcome-unknown; the
adapter returns no receipt and becomes poisoned. A new client plus the exact
command is the only reconciliation path and returns the retained semantic and
persistence receipts if the commit occurred.

## Dependency Direction

```text
lattice-postgres-store
  -> lattice-project-registry
       -> lattice-contracts
       -> lattice-cjson
  -> lattice-task-ledger
  -> lattice-ports
  -> lattice-contracts
  -> lattice-cjson
  -> postgres + sha2 + serde_json
```

There is no reverse domain dependency, Registry-to-Store dependency,
adapter-to-adapter call, Orchestrator dependency, or Contracts/Ports change.

## ADR-016 Amendment

ADR-016 remains authoritative for project-scoped `ControlStore` transactions.
Its statement that every later repository maps into that physical scope is
narrowed: a domain aggregate may instead use a separately versioned typed
global transaction only when its constitution proves that no truthful project
snapshot exists and an ADR freezes its global serialization, evidence, and
compatibility contract. TASK-022 Project Registry is the first and only such
exception approved here. This does not make Store scope optional or generic.

## TASK-075 Schema-v5 Compatibility Amendment

TASK-075 does not change Project Registry 1.2 pure command, checkpoint,
record-set, or persistence-receipt semantics. Schema v4 remains the exact
TASK-022 `db/migrations/0005_project_registry_repository.sql` bytes from
commit `12f7100`, SHA-256
`b7af1f8a8ac370bbfc8a5312497461587cb8a86eb32ff97e5b865c7ae9bf0dcf`.
Its five-entry manifest commitment is
`df3f7ca3687afaa0d1f676158725e6d2f06670e0612df7482aa9d4d244b59f0f`.
Autonomy follows at `0006` as global schema v5; autonomy content at ordinal
`0005` is incompatible history and fails closed before migration DDL without
automatic repair.

Schema v5 adds `persistence_schema_version` and
`persistence_manifest_sha256` to every Registry command row. The migration
backfills all schema-v4 commands with schema `4` and the frozen v4 manifest.
New commands record schema `5` and the current v5 manifest. Reads reconstruct
each persistence receipt from that command-owned profile, not from the
adapter's current profile, so mixed v4/v5 replay remains byte-identical.

The current Registry surface is exactly
`project_registry_prepare_v2`, `project_registry_read_state_v2`,
`project_registry_read_observations_v2`, `project_registry_read_projects_v2`,
`project_registry_read_commands_v2`,
`project_registry_read_reservations_v2`, `project_registry_stage_command_v2`,
`project_registry_stage_project_v2`, and `project_registry_finalize_v2`.
The nine v1 functions remain historical catalog objects with runtime
privileges revoked. Across the base schema-v5 profile, the catalog contains
16 tables and 47 retained functions: 19 are runtime-executable and 28 are
historical non-runtime functions. Extension profiles are accounted for
separately and do not alter these base counts.

## Consequences

- Cross-project collision and reservation behavior has one deterministic
  serialization point and one replayable global command history.
- No fake project/snapshot identity enters durable evidence.
- SQL remains physical defense and cannot invent Registry outcomes.
- Registry loads are globally bounded and potentially more expensive than a
  per-project lookup; correctness is chosen before later snapshot/index
  optimization.
- TASK-022 can prove durable Registry semantics but cannot close AC-06 because
  real Windows/Git inspection, Workspace Git, and Scope Check remain later.

## Rejected Alternatives

- Use the current authority snapshot as `StoreScope`: rejected because
  registration denial has none, transitions rotate it, and valid snapshots may
  exceed the separate Store identifier bound.
- Use a synthetic platform project/snapshot: rejected because the receipt would
  make a false project-snapshot claim and can collide with a real Project ID.
- Use one physical scope per target project plus SQL uniqueness: rejected
  because no single Registry order/checkpoint exists and SQL errors cannot
  deterministically reproduce `Denied` versus `Blocked` semantics.
- Add a global `StoreScope` now: valid but rejected for TASK-022 because it
  requires a larger Contracts/Ports/Store receipt version change without
  improving the bounded Registry evidence.
- Store canonical JSON/blob state: rejected because it hides authoritative
  fields, weakens catalog/ACL validation, and gives SQL no structural defense.
- Let SQL constraints decide collision outcomes: rejected because Project
  Registry is the sole semantic owner and concurrent error order is not a
  stable domain receipt.
