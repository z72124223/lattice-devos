---
module_id: postgres-store
name: LATTICE Postgres Store
version: 1.4
status: active
owner: LATTICE maintainers
last_reviewed: 2026-08-03
---

## Mission

Own the project-scoped physical transaction boundary, deterministic zero-I/O
fake, exact PostgreSQL migration/admission foundation, live durable physical
`ControlStore`, Task-Ledger-planned durable repository adapter, and the one
explicitly governed Project-Registry-planned global persistence exception.
Preserve PostgreSQL as the sole durable control-plane truth and every domain
module as the sole owner of transition legality. `StoreScope` remains strictly
project-scoped; the Registry exception is a separate typed repository contract.

## Non-Goals

- Decide or duplicate Ledger or Registry event/request/receipt/projection/
  reservation/checkpoint semantics; persist Lease, Approval, Artifact, memory,
  provider, or product records; claim/deliver an outbox effect; or claim
  AC-05/19 completion from physical receipts. Version 1.4 may persist only
  Task-Ledger-planned rows, immutable outbox admission for AC-03/04, and the
  Project-Registry-planned global aggregate authorized by TASK-022.
- Add a global variant to Contracts, Ports, `StoreScope`, or Store receipts;
  invent a sentinel `ProjectId`/`ProjectSnapshotId`; reinterpret a project-
  scoped physical head as the global Registry head; or fabricate project
  authority for a registration denial.
- Auto-migrate during normal daemon startup, discover migrations by directory,
  accept edited/applied SQL, or execute the superseded bootstrap draft.
- Create/alter a production login, password, real user database, installed
  PostgreSQL service, credential source, or production role membership. The
  disposable harness may create fixed LOGIN fixtures with its one-time secret.
- Elect a daemon, assign an epoch, or promote bootstrap `STOPPED` admission to
  `ACTIVE`; TASK-019 has no Guardian or normal self-activation path.
- Connect outside an owned loopback disposable PostgreSQL 17 cluster, add TLS,
  or claim remote/cloud compatibility.
- Decide Registry lifecycle, Ledger events/resources, Writer Lease fencing or
  recovery, Approval proof/claim, Artifact quota/retention/sweep, Policy,
  Review Runtime, or Guardian legality.
- Rebuild, reinterpret, or replace a domain receipt/current head.
- Execute an outbox effect or call OpenClaw, Codex, Graphify, Hermes, Git, a
  provider, product code, or a companion/playmate website.
- Claim that physical live durability alone proves One Writer composition,
  Guardian activation, domain repository completeness, effect delivery, or
  release safety.

## Owned Data

- Typed Store transaction envelope, closed repository owner, project/snapshot/
  aggregate scope, physical head, request/transaction/receipt hash subjects,
  terminal fake receipt, and reconciliation identity.
- Physical transaction idempotency used only for exact retry after an unknown
  commit outcome; domain idempotency remains domain-owned.
- Bounded in-memory physical-head and replay maps plus deterministic fake fault
  scripts. These are disposable test state, not durable project truth.
- Exact ordered migration manifest entries: stable ID/path, SHA-256, status,
  transaction mode, and reader/writer compatibility.
- PostgreSQL-owned database identity, exact migration history, schema contract,
  generic future physical-head/terminal-transaction tables, and singleton
  runtime-admission foundation.
- Migration, normal-runtime, future Guardian, and read-only permission contract.
  Each capability remains a fixed `NOLOGIN` role. A distinct fixed LOGIN
  principal is externally provisioned for each capability with exactly one
  `ADMIN FALSE, INHERIT FALSE, SET TRUE` membership and must `SET ROLE`. Before
  that change it has only direct `CONNECT` on the exact target database. The
  test harness proves pre-`SET ROLE` denial using one-time disposable SCRAM
  fixtures. Production provisioning remains external.
- Exact catalog-derived authority closure for external non-system functions,
  all-owner default ACLs, cluster-wide fixed-LOGIN ownership, and a bounded
  protected-function signature manifest. This is verifier evidence, not a new
  function-execution API.
- Server/settings/loopback/role/schema evidence returned by the verifier. It is
  compatibility evidence, not a Store transaction or domain receipt.
- Exact v2 migration-history prefix/upgrade evidence and three fixed runtime
  functions for prepare, finalize, and current-head observation. Function
  execution is the runtime's only physical/terminal table path.
- Live physical heads and terminal transaction rows bound to Store contract,
  producer, runtime, durability, database identity, schema manifest/version,
  exact request, authority, before/after heads, and receipt digests.
- Append-only global migration profiles separated from the immutable Store v2
  receipt profile, so a later database expansion cannot rewrite or invalidate
  an old physical receipt.
- Physical persistence mechanics for Task Ledger stream, command, event,
  projection, checkpoint, and immutable outbox-admission rows produced and
  replay-verified by Task Ledger 2.1.
- The adapter-level PostgreSQL client, transaction, static row conversion,
  bounded retry/poison state, and current global schema-v4 durability evidence
  used by `PostgresTaskLedger`.
- Schema-v4 physical persistence mechanics for one global Registry aggregate:
  singleton state/checkpoint, immutable complete observations, current project
  projections, ordered complete command records, and normalized accepted/
  pending identity reservations. These are exactly the five authoritative
  tables `control.project_registry_state`,
  `control.project_registry_observations`,
  `control.project_registry_projects`,
  `control.project_registry_commands`, and
  `control.project_registry_identity_reservations`.
- Registry adapter transaction, static conversion, bounded retry/poison,
  database/schema identity, and immutable persistence-receipt evidence used by
  `PostgresProjectRegistry`. Project Registry 1.2 remains the owner of every
  semantic request, terminal result, project authority, reservation,
  record-set, and checkpoint.
- The schema-v4 forward-profile runtime allowlist: three Store-v4, five Task-
  Ledger-v2, and nine Project-Registry-v1 fixed functions. Historical function
  definitions remain immutable catalog evidence without runtime EXECUTE.
- The exact schema-v4 catalog closure: 15 `control` tables, 28 retained catalog
  functions, 17 runtime-executable functions, and all 11 historical functions
  retained without runtime EXECUTE.

## Public Contracts

- `ControlStore::transact` accepts one complete typed request and returns one
  terminal fake or live physical receipt, or a typed store error.
- `ControlStore::current_head` returns one independently retained physical
  compare-and-swap head for the exact scope.
- Repository owner is a closed set: Project Registry, Task Ledger, Writer
  Lease, Approval Verifier, or Artifact Store.
- Exact transaction-ID retry returns the identical terminal receipt before
  mutable authority/head checks; changed content under the same ID is denied
  without receipt disclosure or partial mutation.
- The fake checks its retained current daemon authority and physical head,
  admission, revision arithmetic, and capacity before one atomic update.
- Before-apply faults leave no change. After-apply response loss returns an
  unknown outcome; exact retry resolves from the retained receipt.
- Every 1.0 terminal receipt is visibly `Fake` and `NonDurableFake`.
- `MigrationRunner` is an explicit administrative operation over a caller-
  supplied PostgreSQL client. It validates then applies only executable
  manifest entries; normal runtime code does not call it automatically.
- `SchemaVerifier` performs read-only exact manifest, server, settings,
  database identity, bootstrap admission, effective-role, ACL/ownership, and
  protected-function checks.
- The deterministic fake remains visibly non-durable and preserves Store v1
  behavior. Contracts 1.9 / Ports 1.4 add v2 live durability and explicit
  mutable current-head observation without exposing a driver.
- `PostgresControlStore` consumes one caller-supplied already-authenticated
  runtime client and exact disposable target. It reads no environment, DSN,
  password, credential source, or arbitrary SQL.
- The live Store performs prepare and finalize inside one bounded PostgreSQL
  transaction. Exact replay/changed-ID classification precedes admission;
  new writes require exact ACTIVE authority and locked physical head.
- `current_head` observes only one exact scope through the fixed runtime
  function and returns Store-derived live genesis when no row exists.
- `PostgresTaskLedger` consumes one caller-supplied authenticated runtime
  client and exact target, loads only one exact stream through fixed functions,
  delegates all domain construction/replay to Task Ledger 2.1, and returns a
  domain receipt only after the Ledger rows and physical Store receipt commit.
- For new Ledger work at schema v4, Rust sequences fixed `store_finalize_v4`
  followed by fixed `task_ledger_finalize_v2` in the same `SERIALIZABLE`
  transaction. The second function rechecks the base checkpoint and exact
  matching Store terminal; either failure rolls back both, without composite-
  row arguments or additional runtime type/table privileges.
- A successfully appended `EFFECT_INTENT` with audit outcome `RECORDED` creates
  exactly one immutable `ADMITTED` outbox row in the same transaction. Denied,
  non-effect, or appended non-`RECORDED` commands create none; claim/delivery
  is not part of this contract.
- A durable Ledger load returns a pure verified stream plus independently
  matched Ledger checkpoint, physical head, and global schema evidence; no
  database row is authoritative before the pure verifier passes.
- `StoreScope`, `ControlStore`, Store heads, and Store receipts remain strictly
  project/snapshot-scoped. `PostgresProjectRegistry` is the only approved global
  persistence exception; it does not call `PostgresControlStore`, construct a
  `StoreScope`, or return a `StoreTransactionReceipt`.
- `PostgresProjectRegistry` consumes one caller-supplied authenticated runtime
  client and exact verified target, reconstructs the complete global Registry
  through fixed functions, delegates all legality/planning/replay to Project
  Registry 1.2, and returns semantic plus typed Registry persistence evidence
  only after commit and a distinct database/schema/checkpoint match.
- The nine Registry-v1 function roles are fixed: prepare; read singleton state;
  read observations; read projects; read commands; read reservations; stage one
  command plus any new immutable observation; stage one changed project plus
  reservations; and finalize the global checkpoint. They expose no generic
  CRUD, arbitrary row, caller SQL, table, raw client, or migration API.
- Their exact scalar input counts are fixed with no alternate overload:

  | Function | Scalar inputs |
  |---|---:|
  | `project_registry_prepare_v1` | 12 |
  | `project_registry_read_state_v1` | 2 |
  | `project_registry_read_observations_v1` | 2 |
  | `project_registry_read_projects_v1` | 2 |
  | `project_registry_read_commands_v1` | 2 |
  | `project_registry_read_reservations_v1` | 2 |
  | `project_registry_stage_command_v1` | 73 |
  | `project_registry_stage_project_v1` | 22 |
  | `project_registry_finalize_v1` | 27 |

  ADR-020 freezes the complete ordered scalar manifest; the maximum is 73,
  below PostgreSQL's 100-input limit. Composite/table arguments, builtin arrays
  as row maps, JSON payloads, omitted positional fields, and extra runtime type
  privileges are forbidden.
- Project Registry 1.2 owns the canonical command core, logical retained-state
  bytes, checkpoints, record set, and independent retained-checkpoint APIs.
  The adapter stores and independently reads the singleton checkpoint, then
  uses `verify_untrusted_registry_snapshot_against_checkpoint`; plain snapshot
  verification proves only self-consistency and cannot establish currentness.
- Exact Registry command/request replay is classified before current admission
  and returns the retained semantic and persistence receipts without advancing
  the ordinal. Changed command reuse returns no retained receipt. Every new
  terminal command, including `Denied`, `Blocked`, and exact read-only
  observation, advances the global command ordinal/checkpoint exactly once.

## Invariants

1. Every `ControlStore` transaction contains exactly one project, snapshot,
   closed owner, and opaque aggregate scope; mixed/cross-scope substitution
   fails closed. The separately typed Registry transaction is governed only by
   invariants 53 through 70 and never weakens this project-scoped rule.
2. No public request contains SQL, schema/table/column names, filesystem paths,
   arbitrary key/value records, domain-success Booleans, or effect payloads.
3. The fake recomputes the complete canonical request digest; caller-supplied
   digests cannot omit a security-relevant request field.
4. Exact replay precedes mutable authority, admission, physical-head, and
   capacity checks. Changed reuse of an ID never reveals an existing receipt.
5. Current daemon authority is retained independently of the request. Only an
   exact fake `ACTIVE` head admits a normal fake mutation.
6. Physical head equality is complete and independently queried; a bare
   digest or receipt-derived head cannot substitute for currentness.
7. Physical revision zero is exactly the Store-derived deterministic genesis
   for its complete scope; callers cannot seed or override another genesis.
8. Revision increment is checked and never wraps signed PostgreSQL `BIGINT`.
9. Applied head and terminal receipt appear together or neither appears.
10. A stale-head denial does not change the physical head; its exact retry is
   terminal and byte-identical.
11. An unknown after-apply response never reports failure or success; exact
    retry is the only recovery path in this fake.
12. Fake receipt/runtime/durability fields cannot represent live PostgreSQL or
    domain authority.
13. The fake performs no filesystem, network, process, environment, clock,
    randomness, credential, database, provider, product, publication,
    deployment, release, payment, or protected-action I/O.
14. The fake maps are test conformance state only and never become One Truth.
15. Manifest entries are compile-time explicit and bind ordinal, ID, path,
    byte length, SHA-256, status, transaction mode, schema version, and
    compatibility; runtime directory enumeration or arbitrary caller SQL is
    unrepresentable.
16. The exact 312-byte `0001_bootstrap.sql` at SHA-256
    `7BFF021FC17F738551309C906578C8015B2DD0307D27D239C21DF1697C4D09C8`
    is recorded as `SUPERSEDED` and never executed because it owns its own
    transaction and cannot establish exact history/ownership.
17. Before its first SQL mutation, the runner rejects default/wrong database
    names or a missing/mismatched pre-provisioned disposable-run sentinel. It
    then hashes every manifest byte, rejects unknown/missing/reordered/
    mismatched history and unowned pre-existing schemas, and uses one
    transaction-scoped advisory lock and runner-owned transaction.
18. A committed migration and its exact history rows appear together or
    neither appears. An uncertain reply is reconciled by rerunning the same
    manifest and observing exact retained history, never changed SQL. A known
    successful commit followed by verifier failure is separately reported as
    `PostApplyVerificationFailed` (`STORE_MIGRATION_COMMITTED_UNVERIFIED`);
    identical reconnect retry converges to `AlreadyCurrent` only after the
    verifier passes.
19. Migration applies within one read-committed writable transaction under its
    transaction-scoped advisory lock, preserving waiting-runner visibility,
    then after successful commit invokes the same repeatable-read, read-only
    verifier used by normal runtime. Every returned catalog proof uses one
    consistent snapshot. Normal runtime has no direct table DML, DDL, schema
    ownership, effective CREATE on any non-system schema, migration-history/
    identity/admission write, or role escalation. Bootstrap is exactly
    `STOPPED` with no daemon instance/epoch;
    no TASK-019 API promotes it. Future writes require narrow fixed-operation
    procedures that recheck daemon instance/epoch/admission in the same
    transaction; none exists in TASK-019.
20. Driver/server/database failures map to bounded static codes; raw DSNs,
    passwords, SQL values, environment contents, and driver diagnostics never
    enter Debug, Display, receipts, snapshots, or repository files.
21. Live verification mutates only a marker-owned disposable cluster on a
    random loopback port. It never connects to, stops, restarts, migrates, or
    removes the installed PostgreSQL service/data root or a user database.
22. Physical schema tables do not decide domain legality and cannot make the
    migration/verifier result a durable Store/domain receipt.
23. Post-apply/runtime verification checks actual catalog identity, owners,
    constraints, grants, default privileges, column ACLs, dropped-column
    tombstones, inheritance and partition flags; authoritative table reads use
    `ONLY`, owned `pg_type` rows equal the exact table-row/generated-array
    allowlist, and shell or extra types are forbidden. History rows alone never
    authorize a drifted schema. `PUBLIC` has no owned schema/table/function
    authority. Every external non-system function denies effective execution
    to `PUBLIC` and all eight fixed roles.
24. A normal connection authenticates as its fixed LOGIN principal and changes
    current role to exactly one matching `NOLOGIN` capability. Membership is
    exactly `ADMIN FALSE, INHERIT FALSE, SET TRUE`; before `SET ROLE`, the LOGIN
    has only one non-grantable direct `CONNECT` from `lattice_migrator` on the
    exact target database and no database ACL elsewhere. `PUBLIC` has no ACL on
    any database. Before `SET ROLE`, the LOGIN has no capability, database
    `CREATE`/`TEMPORARY`, or direct grant in any PostgreSQL ACL-bearing catalog.
    External non-system relations and columns have no ownership or ACL for
    `PUBLIC` or any of the eight fixed roles. Login attributes, direct grants,
    role settings, and session/current identities are exact; superuser session
    impersonation is not valid runtime evidence.
25. Harness creation fails before I/O if any existing target ancestor is a
    reparse point. Only `pg_ctl status` exit 3 proves stopped; any other error
    preserves the root and fails. PASS follows final stop, cleanup, and installed-
    service equivalence, never precedes them.
26. Database identity is a deterministic domain-separated SHA-256 custom UUIDv8
    bound to the exact target database name and disposable run marker; random,
    zero, malformed, substituted, or standard-UUIDv5-mislabelled identity is
    rejected.
27. `pg_parameter_acl` contains no grant to `PUBLIC` or any fixed capability or
    LOGIN role. Every row in `pg_default_acl`, regardless of recorded owner,
    contains no `PUBLIC` grant. Database, parameter, object, column, type, and
    default-privilege closure is verified from the live catalogs in the same
    repeatable-read snapshot as all other returned evidence.
28. None of the four fixed LOGIN principals owns an object anywhere in the
    cluster. The verifier combines cluster-wide `pg_shdepend` `deptype = 'o'`
    with explicit current-database owner checks so both per-database and shared-
    object ownership fail closed without pretending a local catalog scan sees
    other databases.
29. Before `SET ROLE`, every fixed LOGIN principal is denied effective execution
    of the exact protected-function manifest: `lo_creat(integer)`,
    `lo_create(oid)`, `lo_from_bytea(oid,bytea)`, and both `lo_import`
    overloads; both four-argument `pg_logical_emit_message` overloads; all sixteen
    advisory-lock acquisition overloads; `pg_export_snapshot()`;
    `pg_current_xact_id()`; and `txid_current()`. Two real concurrent sessions
    authenticated as the same fixed LOGIN prove that
    `pg_cancel_backend(integer)` and `pg_terminate_backend(integer,bigint)` are
    also denied. Of the sixteen lock-acquisition overloads, only
    `pg_advisory_xact_lock(bigint)` is granted to `lattice_migrator` after
    `SET ROLE`; no fixed LOGIN has that direct grant. The one allowed direct
    grant is non-grantable and its grantor is the protected function owner.
30. `max_prepared_transactions` is exactly zero. `LISTEN`/`NOTIFY` is never a
    truth, admission, evidence, authority, or effect-delivery source. Because
    `NOTIFY` is a SQL command rather than a function call, function revocation
    is not represented as closing it.
31. Schema v1 upgrades to v2 only when existing history is an exact immutable
    prefix, the complete v1 catalog is verified, and physical/terminal tables
    remain empty. Missing entries plus history/compatibility advance atomically;
    edited, reordered, unknown, partial, or non-empty sources fail closed.
32. Runtime has no direct SELECT/INSERT/UPDATE/DELETE on physical heads or
    terminal transactions. Only exact `control.store_prepare_v2`,
    `control.store_finalize_v2`, and `control.store_current_head_v2` EXECUTE is
    granted to `lattice_runtime`; `PUBLIC`, Guardian, reader, and all fixed
    LOGIN principals have none before `SET ROLE`.
33. Both write-path functions are SECURITY DEFINER, schema-qualified,
    dynamic-SQL-free, non-leakproof, parallel-unsafe, fixed-safe-search-path
    operations owned by `lattice_migrator`. Rust calls prepare/finalize within
    one transaction; finalize revalidates exact authority and locked head even
    if a caller violates that sequencing contract.
34. Exact replay and changed-ID classification precede mutable admission,
    authority, and head checks. Exact retry remains byte-identical after epoch,
    admission, or head change; changed reuse reveals no retained receipt.
35. A new transaction locks and checks exact ACTIVE daemon instance, epoch,
    authority revision/observation/head plus one exact physical scope. Applied
    head and terminal receipt commit together; stale persists a non-mutating
    terminal receipt; checked revision never exceeds signed BIGINT.
36. No durable receipt is returned before successful commit. Only a commit
    failure with no database response is `CommitOutcomeUnknown`; explicit
    database SQLSTATEs retain their known retryable or terminal classification.
    After an unknown outcome, only reconnect plus the exact request may
    reconcile from a retained terminal row. Bounded serialization/deadlock
    retries occur only before an unknown commit outcome.
37. Live durability binds Store v2, fixed producer/runtime/durability,
    database-identity commitment, schema version/manifest commitment, complete
    request, before/after heads, disposition, and digests. It proves no domain
    transition, effect delivery, Guardian activation, or protected action.
38. TASK-020 activation is a test-admin fixture inside the marker-owned
    disposable cluster. No normal runtime or production self-activation API is
    introduced.
39. Global schema v3/full-manifest evidence is distinct from the immutable
    Store receipt profile: Store v2 receipts always bind physical schema profile
    2 and the exact first-three-entry manifest commitment, including after a
    v3 upgrade.
40. Historical `store_*_v2` functions remain catalog evidence but have zero
    runtime EXECUTE at v3. Only the three Store v3 and five Task Ledger v1
    fixed functions are runtime-executable.
41. Task Ledger rows, projection, checkpoint, optional outbox admission, and
    the applied physical Store receipt appear in one transaction or none do.
42. Exact domain command retry and changed-ID classification precede mutable
    admission. Exact replay reconstructs the original physical request and
    receipt even after authority, admission, head, or global schema changes.
43. Every new Ledger terminal command is an applied physical mutation,
    including a domain stale/overflow denial. Such denial leaves the Ledger
    event head unchanged but advances the complete checkpoint and physical
    state exactly once.
44. Only an appended `EFFECT_INTENT` with audit outcome `RECORDED` creates one
    outbox admission whose intent digest equals the event subject digest. Any
    other existing outcome remains append-compatible but creates no admission;
    admission cannot claim or deliver an effect.
45. The complete Task Ledger checkpoint binds identity, head, resource
    projection, ordered events, every command receipt including denials, and
    ordered outbox admissions; it equals the current physical state digest.
46. Domain `u64` values use constrained `numeric(20,0)` and canonical decimal
    text at the SQL boundary. Physical Store revision remains signed `BIGINT`.
47. All authoritative Ledger fields use fixed columns. Only sanitized bounded
    non-authoritative diagnostic data uses `jsonb`; Rust reconstructs and
    validates it before hashing and never hashes PostgreSQL JSON text.
48. Runtime has zero direct SELECT/DML on Ledger/outbox tables. All v3 functions
    are fixed, migrator-owned, SECURITY DEFINER, schema-qualified,
    dynamic-SQL-free, non-leakproof, parallel-unsafe, and safe-search-path.
49. A v2-to-v3 upgrade permits non-empty physical Store history only while the
    exact source is verified and admission is `STOPPED` with no leader. It
    never rewrites a historical physical terminal row.
50. A Store-only mutation that causes a Task Ledger physical/checkpoint
    mismatch cannot manufacture domain rows or authority; the Ledger adapter
    fails closed and never auto-repairs the mismatch.
51. The Task Ledger Store mapping is fixed: scope owner `TASK_LEDGER`, aggregate
    key the complete stream ID, domain command the Ledger request digest,
    record set the plan record-set digest, next state/checkpoint the next
    checkpoint digest, domain receipt the terminal receipt digest, and optional
    outbox intent the Outbox Admission digest rather than its event-subject
    intent digest. The physical transaction ID follows ADR-019's versioned
    owner/stream/command derivation without truncation.
52. Store and Ledger finalization are two ordered fixed-function calls inside
    one transaction, not two commits. At schema v4,
    `task_ledger_finalize_v2` must observe and match the just-created Store
    terminal before writing Ledger rows; failure
    rolls back both and never leaves a Store-only applied terminal.
53. Project Registry is the first and only global persistence exception. It is
    a separately typed repository, not a generic Store scope, and no sentinel,
    synthetic, colliding-project, before, or after project snapshot may stand
    in for the global aggregate.
54. Exactly one `control.project_registry_state` singleton is the global lock,
    serialization, command high-water, count, retained-byte, and checkpoint
    publication point. Migration `0005` seeds it as the Live vacant Registry:
    command high-water and all four counts are zero, retained bytes are 103,
    and checkpoint digest is
    `5bb1f9d9adf7228bef7ea45cc029e79d71761c4dfed3abe086634917db38c173`.
    The other four Registry tables are empty. First-seen command ordinals are
    the strict positive sequence `1..N`; the non-negative high-water is signed-
    `BIGINT`-compatible, monotonic, and non-wrapping.
55. The complete Registry checkpoint binds runtime, every first-seen command in
    ordinal order, all current project projections, accepted and pending
    reservations, deterministic counts, retained-byte accounting, and the
    command checkpoint chain. Project Registry constructs both the replayed
    checkpoint and `RegistryCheckpoint::from_retained`; the adapter must compare
    every projection plus the independently read singleton through
    `verify_untrusted_registry_snapshot_against_checkpoint` before returning
    authority. Self-consistency without that independent comparison is not
    currentness and cannot reject a removed denial/observation tail.
56. Schema v4 has exactly the five normalized Registry tables named in Owned
    Data. Complete observations are immutable and digest-keyed; identity
    reservations are normalized accepted/pending corruption defenses. No
    Registry request, observation, denial, authority, drift, reservation,
    checkpoint, receipt, or authoritative state is `jsonb`, an arbitrary map,
    an opaque canonical blob, or caller-defined SQL.
57. Pure Project Registry 1.2 alone decides registration, observation,
    suspension, collision blocking, reconciliation, reservations, terminal
    receipts, record sets, and checkpoints. SQL uniqueness and constraints may
    detect corruption but cannot construct `Denied`, `Blocked`, or authority.
58. Exact command/request replay and changed-ID classification precede capacity
    and mutable admission checks. Every first-seen terminal command advances
    the global ordinal/checkpoint once even when it changes no project; only a
    legal lifecycle mutation advances the target project's revision/authority.
59. Registry-owned limits fail closed before mutation: at most 4,096 current
    project projections, 65,536 first-seen terminal commands, 67,108,864
    retained snapshot bytes, and 131,072 UTF-8 bytes in one already-NFC
    canonical root. Version 1.4 adds no deletion, compaction, or truncation.
60. New Registry work uses one bounded `SERIALIZABLE` transaction. It checks the
    exact ACTIVE daemon authority, admission, forward global profile, and locked
    base checkpoint; fixed staging plus checkpoint publication commits all rows
    and immutable persistence evidence together or none. PostgreSQL uses fixed
    5-second lock, 30-second statement, and 30-second idle-in-transaction
    timeouts; Rust enforces a 45-second monotonic begin-to-pre-commit deadline.
61. Registry finalization accepts only stage rows whose `xmin` belongs to the
    current transaction. It rechecks the exact base/result checkpoint,
    request/result/record-set shape, counts, and allowed project/reservation
    replacement before publishing the singleton result checkpoint.
62. A deliberately committed stage without finalization is never authority.
    Extra, missing, reordered, duplicated, injected, orphaned, or partial
    observation/project/command/reservation rows make the retained state
    corrupt; the adapter fails closed and never repairs, completes, or deletes
    them silently.
63. Schema v4 has exactly 15 `control` tables, 28 retained catalog functions,
    and 17 runtime-executable functions: three `store_*_v4`, five
    `task_ledger_*_v2`, and nine `project_registry_*_v1`. All 11 historical
    functions remain present without runtime EXECUTE. Every executable function
    is exact-signature, migrator-owned, `SECURITY DEFINER`, schema-qualified,
    dynamic-SQL-free, non-leakproof, parallel-unsafe, row-security-on, safe-
    search-path, `lock_timeout = 5s`, and `statement_timeout = 30s`; runtime has
    zero direct SELECT/DML on protected tables.
64. Migrations `0001` through `0004` remain byte-identical. The runner accepts
    only Fresh or exact v1/v2/v3/v4 profiles; exact transaction-control-free
    `0005` advances only the exact v3 prefix, requires a non-empty v3 source to
    be `STOPPED` with no leader, seeds exactly the Live vacant singleton defined
    by invariant 54, leaves observations/projects/commands/reservations empty,
    and never rewrites a historical Store or Ledger terminal. Exact v4 is a
    verified no-op.
65. Store-v2 receipt profile 2 and its first-three-entry manifest commitment
    remain byte-identical after schema-v4 expansion. Store-v3 and Task-Ledger-
    v1 functions remain ungranted catalog history; successor Store-v4 and
    Ledger-v2 functions preserve historical semantic and receipt replay while
    binding the constructor-frozen current global profile.
66. No durable Registry receipt is returned before commit. Only commit failure
    with no database response is outcome-unknown and poisons the adapter; a new
    client plus the exact request is the sole reconciliation path. Explicit
    responses remain known retryable or terminal outcomes, and bounded
    serialization/deadlock retries occur only before outcome uncertainty.
    Lock timeout, statement timeout, idle-in-transaction timeout, and the Rust
    pre-commit deadline roll back and return typed `Unavailable`; they are not
    commit-unknown and receive no automatic retry.
67. Project Registry's canonical construction order is mandatory: checkpoint
    command core first; logical retained state and retained-byte count second;
    result checkpoint third; record set over the command persistence core plus
    optional new observation/project/reservation deltas fourth; Registry
    transaction digest fifth; adapter persistence receipt last. Checkpoint
    cores exclude checkpoint references, record-set/count and adapter fields;
    record sets exclude their own digest and every adapter/database field.
68. Registry-owned retained-byte accounting is exactly the byte length of the
    `lattice.project-registry.logical-retained-state` schema version `1`
    `lattice-cjson-1`
    object containing schema version, runtime, digest-sorted observations,
    Project-ID-sorted projects, strict-ordinal command cores, and identity/
    status/project-sorted reservations. Observations are counted once, optional
    fields are explicit `null`, text is NFC UTF-8, and unsigned values are
    canonical decimal strings. Hash frames, checkpoint references/digests/counts,
    record-set digests, SQL overhead, and adapter/database/schema/transaction/
    persistence fields are excluded. SQL compares but never computes this
    domain accounting.
69. The nine Registry functions have scalar input counts `12, 2, 2, 2, 2, 2,
    73, 22, 27` in the exact name/order manifest in Public Contracts. The
    73-input stage-command function is the maximum. No composite/table argument,
    builtin array row map, JSON payload, omitted position, alternate overload,
    or extra runtime type privilege may reduce or reinterpret that surface.
70. Rust checks its 45-second monotonic deadline after every read batch, after
    pure replay, and before staging, finalization, and commit. Exceeding it or
    the 30-second idle-in-transaction limit is a known pre-commit
    `Unavailable`, rolls back, returns no receipt, and is never automatically
    retried.

## Allowed Dependencies

- `lattice-contracts` 1.9.
- `lattice-ports` 1.4.
- `lattice-cjson` 1.0.
- `lattice-task-ledger` 2.1 pure planner/checkpoint/replay API.
- `lattice-project-registry` 1.2 pure planner/checkpoint/replay API, one-way
  from this adapter only.
- Exact `postgres` 0.19.14 with default features disabled.
- Exact `sha2` 0.11.0.
- Exact `serde_json` 1.0.151 used only to convert the bounded diagnostic `jsonb`
  value back into `CanonicalValue`; PostgreSQL JSON text is never hash input.
- Rust standard-library in-memory collections and errors.

## Forbidden Dependencies

- SQLx, Diesel, direct `tokio-postgres`, pools, TLS adapters, libpq, dynamic SQL
  or migration discovery, environment/credential loaders, provider/model SDKs,
  Git, product repositories, and companion/playmate website code.
- Task Domain, Writer Lease, Approval, Artifact, Policy,
  Orchestrator, Gateway, another concrete adapter, Review Runtime, Codebase
  Memory, or Guardian crates. Task Ledger 2.1 and Project Registry 1.2 are the
  only approved domain-owner dependencies in version 1.4.
- Any adapter-to-adapter dependency or reverse dependency from a domain owner.

## Failure, Compatibility, And Migration

The 1.0 fake remains a zero-I/O conformance boundary. Malformed/substituted,
unauthorized, stale, overflowed, capacity-exhausted, unavailable,
serialization-exhausted, corrupted, and outcome-unknown states are distinct
typed outcomes; none imply success. The fake itself makes no live PostgreSQL
compatibility or durability claim.

Version 1.1 adds only a PostgreSQL 17 schema/migration/admission compatibility
foundation. Migration errors distinguish manifest/checksum/history/order/
ownership/lock/transaction/compatibility/permission/availability/unknown
outcomes with bounded codes. Version 1.1.3 additionally distinguishes an
unknown commit outcome from a known commit whose post-apply verifier failed;
neither implies verified success. The runtime verifier is read-only. At 1.1,
a live `ControlStore` and durable receipt remained deferred until the
versioned TASK-020 expansion and direct transaction/restart evidence.

Version 1.1.4 adds fail-closed effective function/default-ACL/cluster ownership
closure, the exact pre-`SET ROLE` protected-function manifest, real same-LOGIN
backend-control denial, and the zero-prepared-transaction/non-authoritative-
notification boundary. It does not add a runtime callable function surface.

Version 1.2 adds an exact v1-to-v2 expansion migration, a three-function
runtime allowlist, and the live durable physical Store. It does not add a domain
repository, production provisioning, leader activation, remote/TLS connection,
provider/product effect, or release authority.

Version 1.3 adds exact global schema v3, a frozen historical Store receipt
profile, five narrow Task Ledger persistence functions, and
`PostgresTaskLedger`. It completes only durable Task Ledger append/outbox
admission/restart replay. Live resource observation, outbox claim/delivery,
other domain repositories, activation, production connectivity, and protected
release remain deferred.

Version 1.4 adds exact global schema v4 and the one Registry-specific global
persistence exception through `PostgresProjectRegistry`. It consumes Project
Registry 1.2's pure planner/verifier, adds five normalized authoritative tables
including immutable observations, and replaces the runtime surface with three
Store-v4, five Ledger-v2, and nine Registry-v1 fixed functions while preserving
historical Store/Ledger receipt meaning. Its corrected governance freezes the
Live vacant singleton, Registry-owned acyclic commitment/retained-byte/current-
checkpoint semantics, exact `15 / 28 / 17 / 11-ungranted` catalog closure, the
nine-function scalar parameter budget, and the combined PostgreSQL/Rust
transaction bounds. Contracts 1.9, Ports 1.4, project-scoped `StoreScope`, live
Windows/Git inspection, other domain repositories, activation, production
connectivity, providers/products, effect delivery, and protected release do
not change.

## Acceptance Gates

| Gate | Evidence | Owner | Required for merge |
|---|---|---|---|
| Contract/port/fake matrices | focused Rust tests | Engineering | yes |
| Scope and substitution | project/snapshot/owner/aggregate/transaction/authority/head field matrix | Security review | yes |
| Atomicity and retry | apply, stale, exact retry, changed ID, before/after fault tests | Engineering | yes |
| Fake truthfulness | fixed Fake/NonDurableFake receipt assertions | Architecture review | yes |
| Bounds/capacity/corruption | exact-bound, plus-one, overflow, retained-replay and corruption tests | Security review | yes |
| Dependency/no-I/O | Cargo tree plus forbidden source/dependency scan | Architecture review | yes |
| Migration inactivity | migration file hash unchanged; no runner/driver/connection | Integration review | yes |
| Full verification | workspace format, strict Clippy, Rust/Node/governance tests | Engineering | yes |
| Manifest closure | explicit ordinal/ID/path/length/status/checksum/schema/compatibility matrices; no directory discovery | Security review | yes |
| Transactional migration | apply/no-op/concurrent runner/rollback/unknown-response and committed-unverified reconciliation | Engineering | yes |
| Ownership and roles | exact target sentinel, cluster-wide database/PUBLIC/parameter ACL closure, all-catalog LOGIN direct grants, cluster-wide `pg_shdepend` plus explicit local fixed-LOGIN owner checks, external relation/column/effective-function closure, all-owner default-ACL closure, and migrator/runtime/guardian/reader matrix | Security review | yes |
| Catalog and type closure | exact column ACL signature, table-row/array type allowlist, shell/extra type denial | Architecture review | yes |
| Protected function/settings closure | exact 5 large-object creator, 2 four-argument logical-message, 16 advisory-lock, same-LOGIN backend-control, snapshot-export, and transaction-ID function proof; only one exact non-grantable post-`SET ROLE` migrator advisory-lock grant from the function owner; `max_prepared_transactions = 0`; no notification-authority claim | Security review | yes |
| Admission bootstrap | exact STOPPED/no-leader row; runtime cannot mutate or self-activate | Architecture review | yes |
| Disposable PostgreSQL | owned isolated 17.10 cluster, loopback/settings/checksums/restart/cleanup evidence | Integration review | yes |
| Secret/error hygiene | redacted static errors and zero credential/DSN persistence | Security review | yes |
| Exact expansion migration | fresh v2 plus verified v1 prefix upgrade, rollback, concurrent runner, and drift matrices | Integration review | yes |
| Live physical transaction | apply/stale/replay/substitution/authority/head/isolation/overflow matrices | Engineering | yes |
| Durable reconciliation | commit-response loss, reconnect exact retry, restart, and corrupt-row denial | Security review | yes |
| Runtime function ACL | exact function signature/source/property/grant manifest plus direct table denial | Security review | yes |
| Historical Store profile | v2 receipt before/after v3 upgrade replays byte-identically; old v2 functions become ungranted | Compatibility review | yes |
| Ledger planner parity | fake and PostgreSQL use Task Ledger 2.1 plan/checkpoint/replay; no duplicated domain builder | Architecture review | yes |
| Ledger atomicity | command plus optional event/outbox, projection/checkpoint, and physical receipt all-or-none | Engineering | yes |
| Ledger restart/corruption | restart replay plus event/command/denial/outbox/head/checkpoint/physical mismatch matrices | Security review | yes |
| Ledger concurrency/reconciliation | same command, same stream, cross-stream, bounded retry, response loss, reconnect replay | Engineering | yes |
| Schema-v3 migration | fresh, v1, non-empty stopped v2, v3 no-op, rollback/concurrent runner, ACTIVE denial | Integration review | yes |
| Registry planner parity | Fake and PostgreSQL use Project Registry 1.2 vacant/plan/apply/checkpoint/replay; Registry 1.1 observation/request/authority-receipt/command-result golden vectors stay byte-identical, while vacant/non-vacant checkpoint and record-set vectors are new in 1.2 | Architecture review | yes |
| Registry global-scope truth | no `StoreScope`, synthetic project/snapshot, Store receipt, or fabricated denial authority; no Contracts/Ports change | Security review | yes |
| Registry schema normalization | exact singleton plus immutable observation/project/command/reservation five-table catalog; fixed fields and no authoritative JSON/blob | Architecture review | yes |
| Registry vacant/current checkpoint | `0005` seeds the exact Live vacant singleton (`0` high-water/counts, 103 bytes, frozen digest), other tables empty, commands `1..N`, and adapter compares replay with independently read checkpoint | Compatibility review | yes |
| Registry atomicity/current transaction | SERIALIZABLE prepare/read/stage/finalize, current-`xmin` provenance, all-or-none checkpoint/rows/evidence, partial-stage denial, 5s/30s/30s PostgreSQL bounds plus 45s Rust pre-commit deadline | Engineering | yes |
| Registry canonical/current replay | acyclic command-core/logical-state/checkpoint/record-set/transaction/receipt construction; exact retained-byte algorithm; self-consistency versus independent-current-checkpoint denial-tail matrices | Security review | yes |
| Registry replay/corruption | denial tail, reorder, duplicate, injection, substitution, orphan/missing/extra observation, project, command, reservation, count, runtime and checkpoint matrices | Security review | yes |
| Registry concurrency/reconciliation | same command/project, cross-project identity collision, pending front-run, blocking, unrelated registration, bounded retry, commit-response loss/reconnect | Engineering | yes |
| Registry bounds | exact and plus-one 4,096-project, 65,536-command, 67,108,864-byte retained-state, and 131,072-byte canonical-root matrices | Security review | yes |
| Schema-v4 migration and function profile | fresh/v1/v2/non-empty stopped-v3/v4, rollback/concurrent/ACTIVE; exact 15 tables/28 catalog functions/17 runtime functions/11 historical-ungranted; Registry scalar counts 12, five reads at 2, 73, 22, 27 with max 73 and no composite/array/JSON escape | Integration review | yes |
| Historical Store/Ledger compatibility | `0001`-`0004` byte-identical; Store-v2 receipts and existing Ledger receipts/checkpoints replay identically after v4 | Compatibility review | yes |

## Change Policy

Mission, owner set, transaction fields/order, digest subjects, receipt or
durability meaning, trait signatures, migration behavior, dependencies, or
failure semantics require a versioned constitution amendment, SPEC/ADR trace,
architecture review, and authorization consistent with protected-action rules.

## Amendment History

| Version | Date | Decision reference | Summary | Approver |
|---|---|---|---|---|
| 1.0 | 2026-08-01 | SPEC-002 v15, ADR-016, TASK-018 | Typed zero-I/O physical transaction contract and visibly non-durable deterministic fake | User MVP-3 execution directive |
| 1.1 | 2026-08-01 | SPEC-002 v16, ADR-017, TASK-019 | Initial exact-manifest PostgreSQL 17 schema, role, compatibility, and STOPPED-admission foundation | User MVP-3 execution directive |
| 1.1.1 | 2026-08-01 | SPEC-002 v17, ADR-017 review correction, TASK-019 | Fixed real-login capability mapping, consistent catalog snapshot/inheritance closure, and fail-closed harness creation/cleanup evidence; live ControlStore remains deferred | User MVP-3 execution directive |
| 1.1.2 | 2026-08-01 | SPEC-002 v18, ADR-017 review correction, TASK-019 | Non-inheriting LOGIN mapping with exact CONNECT-only bootstrap, repeatable-read apply verification, custom UUIDv8 identity, and fail-closed harness evidence | User MVP-3 execution directive |
| 1.1.3 | 2026-08-01 | SPEC-002 v19, ADR-017 review correction, TASK-019 | Cluster-wide database/PUBLIC/parameter ACL closure, all-catalog LOGIN and external relation/column closure, exact owned type closure, and committed-unverified retry semantics | User MVP-3 execution directive |
| 1.1.4 | 2026-08-02 | SPEC-002 v20, ADR-017 review correction, TASK-019 | Effective external-function and all-owner default-ACL closure, cluster-wide fixed-LOGIN ownership closure, exact protected-function manifest, zero prepared transactions, and non-authoritative LISTEN/NOTIFY boundary | User MVP-3 execution directive |
| 1.1.5 | 2026-08-02 | SPEC-002 v21, ADR-017 review correction, TASK-019 | Correct the protected manifest to all five large-object creators and PostgreSQL 17 four-argument logical-message identities; require exact grant-option and grantor closure | User MVP-3 execution directive |
| 1.2 | 2026-08-02 | SPEC-002 v22, ADR-018, TASK-020 | Exact schema-v1-to-v2 expansion plus narrow function-gated live durable physical ControlStore | User MVP-3 execution directive |
| 1.3 | 2026-08-02 | SPEC-002 v23, ADR-019, TASK-021 | Global schema-v3 profile, immutable Store-v2 receipt compatibility, and Task-Ledger-planned durable event/receipt/projection/outbox repository | Approved V2 amendment and user MVP-3 execution directive |
| 1.4 | 2026-08-03 | SPEC-002 v24, ADR-020, TASK-022 | Global schema-v4 profile and sole typed Registry persistence exception, corrected with the seeded vacant singleton, Registry-owned canonical/current checkpoint, exact catalog/signature budgets, and bounded transaction semantics without changing Contracts/Ports | Approved V2 amendment and user MVP-3 execution directive |
