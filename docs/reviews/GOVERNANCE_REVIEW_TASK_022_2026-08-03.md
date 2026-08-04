# TASK-022 Independent Governance Review

## Decision

`CHANGES REQUIRED — IMPLEMENTATION BLOCKED`.

The final TASK-022 governance set has a coherent architectural direction, but
it does not yet freeze an implementable canonical commitment graph, vacant
state, rollback anchor, retained-byte algorithm, or fixed-function parameter
budget. Rust and SQL implementation must not start until the P1 findings below
are corrected and the corrected governance set receives a new independent
review.

| Severity | Count |
|---|---:|
| P0 | 0 |
| P1 | 5 |
| P2 | 4 |
| P3 | 0 |

Implementation blocker: **not released**.

## Review Scope And Independence

This was a read-only architecture, module-boundary, compatibility, and
implementability review of:

- `PLANS.md`;
- SPEC-002 version 24, including AC-36;
- TASK-022;
- ADR-020 and the ADR-016 amendment;
- Project Registry 1.2 and Postgres Store 1.4 constitutions;
- `docs/modules/V2_AMENDMENT_PROPOSAL.md` and `docs/modules/README.md`;
- the TASK-022 workflow audit;
- the current Project Registry 1.1 implementation, shared `StoreScope`, Task
  Ledger 2.1 checkpoint/replay precedent, schema-v3 Store/Task-Ledger adapter,
  and immutable `0004` migration contract.

No reviewed governance file, Rust source, SQL migration, Git state, database,
credential, service, or external system was modified. This report is the only
file added by the review. No Rust or PostgreSQL acceptance suite was run.

## P1 Findings

### P1-1 — The canonical commitment graph is not frozen and can self-reference

ADR-020 defines a complete command record with base/result checkpoints
(`ADR-020:53-55`), a checkpoint over the complete first-seen semantic command
history (`ADR-020:75-90`), and a record-set digest over one new command record
plus project/reservation replacement (`ADR-020:87-88`). The physical command
row then contains the checkpoint chain, record set, and Registry persistence
receipt (`ADR-020:162-165`), while that persistence receipt binds the result
checkpoint and record-set digest (`ADR-020:120-132`).

Only checkpoint-reference fields are expressly excluded from the checkpoint's
own canonical input. The documents do not freeze:

- whether the checkpoint command projection includes `record_set_digest` or
  PostgreSQL persistence evidence;
- whether the record-set command projection excludes its own digest,
  persistence receipt, transaction digest, and database/schema evidence;
- whether the optional newly retained immutable observation staged by
  `ADR-020:217-219` is a record-set member; or
- the construction order among result checkpoint, record set, transaction
  digest, persistence receipt, and retained-byte accounting.

As written, a complete physical command row can make the record set include its
own digest or persistence receipt, while the persistence receipt includes the
record set and result checkpoint. That is not a deterministic hash input.

The existing Task Ledger demonstrates the missing separation:
`crates/lattice-task-ledger/src/lib.rs:2980-3017` defines distinct command-core
and persistence projections, and `:3205-3209` explicitly excludes checkpoint
references from the checkpoint input to break the cycle.

Required correction: freeze separately versioned canonical projections and
their construction order. At minimum, distinguish (1) checkpoint command core,
(2) record-set persistence core including every optional staged semantic row
but excluding its own and adapter digests, and (3) adapter persistence evidence
constructed last.

### P1-2 — Retained-byte accounting has a limit but no reproducible algorithm

SPEC-002 `:374-380` and `:1141-1159`, ADR-020 `:100-112`, Project Registry
1.2 invariants `:153-159`, and Postgres Store 1.4 invariants `:413-416` all
freeze the 67,108,864-byte limit and require exact boundary/plus-one evidence.
None freezes the byte-accounting subject.

The governance set does not decide:

- which observation, project, command, reservation, metadata, and optional
  fields are counted;
- whether one digest-keyed observation referenced by several commands is
  counted once or once per reference;
- whether the measure is canonical `lattice-cjson-1` bytes, UTF-8 field bytes,
  SQL storage bytes, or another framing;
- the canonical collection order and presence/null framing; or
- whether checkpoints, counts, record-set digests, persistence receipts, and
  database/schema evidence are included or excluded.

Including derived checkpoint or record-set fields can also reintroduce the
cycle in P1-1. Fake and PostgreSQL can therefore produce different legal counts,
and the required exact 64-MiB tests have no single expected result.

Required correction: Project Registry 1.2 must own a versioned canonical
logical-byte algorithm, including exact included projections, ordering,
deduplication, null framing, UTF-8 treatment, and derived/adapter-field
exclusions. SQL may compare the resulting count but must not invent it.

### P1-3 — Denial-tail rollback cannot be detected without an independent current checkpoint

Project Registry 1.2 exports observations, projects, commands, reservations,
accounting, and checkpoint together as untrusted state
(`MODULE_CONSTITUTION.md:66-72`) and requires denial-tail rollback to fail
closed (`:145-152`; TASK-022 `:86-89`). Replaying a truncated command prefix
from vacant state and supplying that prefix's matching old checkpoint is,
however, a valid self-contained old state. Pure replay cannot know that a later
denial or no-project-change command once existed.

ADR-020 `:58-60` mentions comparison with an independently stored checkpoint,
but no public contract freezes how that checkpoint is reconstructed, supplied,
or compared. The acceptance criteria do not require the verifier to receive an
independent retained checkpoint.

Task Ledger 2.1 already provides the required precedent:
`LedgerCheckpoint::from_retained` at
`crates/lattice-task-ledger/src/lib.rs:899-924` and
`verify_untrusted_snapshot_against_checkpoint` at `:1633-1647`.

Required correction: freeze an equivalent Registry retained-checkpoint type
and verification API, require PostgreSQL loads to compare replay with the
independently read singleton checkpoint, and distinguish self-consistency from
currentness/rollback protection.

### P1-4 — Vacant state, positive ordinal, singleton creation, and first-write locking conflict

The domain must construct a vacant Registry (`ADR-020:44-60`), but the
checkpoint ordinal is required to be positive (`:78-85`) and the SQL global
ordinal remains positive (`:171-175`; Postgres Store 1.4 `:390-393`). Project
Registry 1.2 separately and more narrowly requires each retained command record
ordinal to be non-zero (`MODULE_CONSTITUTION.md:73-75`).

At the same time, the schema defines one singleton state row
(`ADR-020:152-154`), new work must lock that row (`:211-212`), and the v3-to-v4
upgrade is described as adding empty Registry tables (`:235-239`; Postgres
Store 1.4 `:436-440`). The governance set therefore does not establish whether
`0005` seeds a vacant singleton, how two concurrent first commands acquire one
serialization point if it does not, or how a zero-command high-water can obey a
positive-ordinal constraint.

Required correction: freeze the vacant checkpoint fields and digest, whether
the singleton is seeded by `0005`, the first-write lock protocol, and the
ordinal representation. One implementable model is high-water `0` for vacant
state and positive command ordinals `1..N`, but the responsible governance
documents must make the choice consistently.

### P1-5 — The nine-function profile has no proven PostgreSQL parameter budget

ADR-020 requires fixed authoritative columns (`:171-175`), exactly nine
Registry functions with one command-plus-observation stage function
(`:186-194`, `:217-219`), and rejects composite/table arguments, array row maps,
JSON payloads, extra type privileges, and similar parameter-reduction escape
hatches (`:225-227`). Every function must also have an exact verified signature
(`:196-201`).

The governance set lists function roles, but no scalar signatures, complete
table-column derivation, shared-field derivation, or parameter count. The
current, smaller Task Ledger finalizer already has 70 scalar input parameters
(`crates/lattice-postgres-store/src/postgres_setup.rs:314`). The proposed
Registry stage must represent a complete typed request, before/after authority,
semantic result, checkpoint chain, record set, daemon authority, profile/
database evidence, persistence evidence, and optional complete observation.

The review cannot prove that the stage is over PostgreSQL's 100-argument limit,
but the governance claim that the chosen split avoids it is also unproven. If
implementation discovers an overrun, the frozen exactly-nine-function contract
would already be invalid.

Required correction: freeze all nine scalar signatures, each parameter's
source or deterministic SQL derivation, and the exact input count. If the stage
cannot remain within 100 inputs without a prohibited representation, revise
the function count and all dependent governance before implementation.

## P2 Findings

### P2-1 — Golden-parity terminology claims historical digests that do not exist

SPEC-002 `:381-384` and `:1154-1156`, TASK-022 `:76-78`, Project Registry 1.2
`:160-163`, the V2 amendment `:522-526`, and the modules README `:128-132`
describe TASK-012/Project Registry 1.1 request, authority, result, record-set,
and terminal-receipt digests as byte-identical historical vectors.

The current Registry contains zero `record_set` occurrences across its source,
tests, TASK-012, and ADR-010. `RegistryCommandReceipt` has only `command_id`,
`request_digest`, before/after heads, outcome, drift, optional authority, and
`result_digest` (`crates/lattice-project-registry/src/lib.rs:508-517`). Its
existing hash subjects are repository observation, command request, authority
receipt, and command result (`:156-165`, `:442-503`, `:1179-1223`,
`:1246-1280`); there is no historical record-set subject or separately exposed
terminal-receipt digest.

Required correction: name the actual 1.1 golden set precisely. Treat checkpoint
and record-set subjects as new 1.2 vectors. Define whether "terminal receipt
digest" means the existing command `result_digest`; if it is distinct, version
it as a new subject rather than claiming historical byte identity.

### P2-2 — The transaction is not bounded across the Rust replay interval

ADR-020 calls the operation one bounded `SERIALIZABLE` transaction
(`:203-223`) but freezes only `lock_timeout = 5s` and
`statement_timeout = 30s` (`:196-201`). The adapter locks one global row, reads
up to the 64-MiB retained domain state, and performs Rust reconstruction/replay
before staging and finalization. PostgreSQL statement timeout does not bound
client-side Rust work between statements, and no total deadline or
idle-in-transaction limit is frozen.

Required correction: define a total/idle transaction bound and its typed
failure/retry classification, or move unbounded replay outside the lock and
freeze a finalize-time checkpoint revalidation protocol that preserves the
single legal history.

### P2-3 — Exact schema-v4 total catalog counts are not frozen

The Registry-specific count is consistent across the final documents: five new
Registry tables and nine Registry functions. The complete schema-v4 catalog is
larger because successor Store and Ledger functions are added while historical
definitions remain.

Current executable history contains 10 `control` tables and 11 functions.
Therefore the proposed exact profile is:

- 5 new Registry tables and 15 total `control` tables;
- 17 new functions: 3 Store-v4 + 5 Task-Ledger-v2 + 9 Registry-v1;
- 28 total catalog functions after retaining the 11 historical functions;
- exactly 17 runtime-executable functions; and
- all 11 historical functions retained without runtime EXECUTE.

Postgres Store 1.4 `:430-445` freezes the executable groups and historical
behavior, but not these total verifier counts. Because the existing schema
verifier uses exact total function counts
(`crates/lattice-postgres-store/src/postgres_setup.rs:1718-1726`), the v4
catalog gate should freeze `15 / 28 / 17 / 11-ungranted` explicitly and avoid
interpreting "five tables/nine functions" as the whole migration delta.

### P2-4 — The workflow-audit version classification is stale for this review gate

The workflow audit is explicit that it records an earlier snapshot, but it
still says the current files are SPEC-002 v23, Project Registry 1.1, and
Postgres Store 1.3 (`WORKFLOW_AUDIT_TASK_022_2026-08-03.md:18-21`) and classifies
the spec, constitutions, and ADR as partial at `:99-101`. The files reviewed now
are SPEC-002 v24, Project Registry 1.2, and Postgres Store 1.4.

The historical observation is not rewritten by this review, but it cannot serve
as current gate evidence for PLANS `:595-607`, which requires the workflow audit
and final governance set to agree before implementation. Refresh the audit's
current-stage classification after resolving the findings above.

## Cross-Contract Results Without Findings

The following reviewed boundaries are internally aligned and do not create an
additional finding:

- **Global versus project scope:** ADR-016 `:141-149`, ADR-020 `:278-286`, both
  constitutions, SPEC-002, and TASK-022 consistently keep `StoreScope`,
  `ProjectSnapshotId`, `ControlStore`, and Store receipts project-scoped. The
  Registry adapter is a distinct typed global exception and dependency remains
  one-way `lattice-postgres-store -> lattice-project-registry`.
- **Immutable observations and domain ownership:** current
  `RepositoryObservation` fields are private and its digest is reconstructed
  from complete identity inputs (`lattice-project-registry/src/lib.rs:114-210`).
  The five-table design keeps SQL uniqueness as corruption defense and leaves
  `Denied`, `Blocked`, lifecycle, revision, and reservation meaning in pure
  Project Registry.
- **Five/nine role count:** the five named Registry tables and nine Registry
  function roles are counted consistently. Prepare + five reads + two stage
  functions + finalize equals nine. P1-5 concerns the missing signatures and
  parameter proof, not this arithmetic.
- **StoreScope code contract:** current `StoreScope` requires exact project and
  immutable snapshot identity (`lattice-contracts/src/lib.rs:7872-7931`), so
  the decision not to overload it is compatible with the existing public
  contract. Contracts 1.9 and Ports 1.4 need no TASK-022 change.
- **Schema-v4 compatibility direction:** preserving `0001` through `0004`, the
  immutable Store-v2 receipt profile, historical Store/Task-Ledger rows, and
  successor Store-v4/Ledger-v2 availability is compatible with the current v3
  manifest and Task Ledger adapter. No old row rewrite is required.
- **Current-transaction and ACL direction:** current Ledger finalization already
  checks Store-terminal `xmin` against `pg_current_xact_id()` inside a
  migrator-owned security-definer function
  (`0004_task_ledger_repository.sql:2316`). The proposed Registry use can retain
  runtime denial of direct protected-table and protected-function access.
- **Commit uncertainty:** ADR-020, Postgres Store 1.4, and the existing Store/
  Ledger adapters consistently reserve `CommitOutcomeUnknown` for commit
  failure with no database response. Explicit SQLSTATE responses remain known
  retryable or terminal outcomes; exact retry on a new client is the only
  reconciliation path.
- **Scope and non-goals:** the reviewed set consistently excludes live
  Windows/Git inspection, Workspace Git, Scope Check, later repositories,
  external components, production, release/deployment, credentials, primary
  merge, and the unrelated companion/playmate website.

## Architecture Impact

| Boundary | Current verified code | Intended TASK-022 state | Review result |
|---|---|---|---|
| Project Registry | pure Fake owner; no checkpoint/export/record-set surface | shared Fake/Live vacant/plan/apply/export/verify | direction valid; five P1 contracts must be frozen first |
| Persistence scope | project/snapshot `StoreScope` | separately typed global Registry transaction | aligned |
| Database profile | schema v3, durable Store + Task Ledger | schema v4, five Registry tables, successor runtime profile | additive compatibility direction valid; exact total catalog gate needs P2 correction |
| Authority | pure Registry semantic receipts; Store/Ledger durable evidence | semantic receipt plus distinct Registry persistence receipt after commit | aligned if P1-1 projections are separated |
| Replay/currentness | Task Ledger has independent retained-checkpoint comparison | Registry promises denial-tail rollback rejection | P1-3 blocks implementation |

No dependency cycle, reverse adapter dependency, second truth/writer, generic
SQL escape hatch, or false project scope is introduced by the proposed
direction.

## Verification Evidence

- `npm.cmd run check`: PASS; `check=ok files=270 constitutions=18 tickets=22
  current_tasks=1`.
- Current source/contract scan: zero `record_set` matches in Project Registry
  1.1 source, tests, TASK-012, or ADR-010.
- Existing Task-Ledger-v1 finalizer signature: 70 scalar inputs.
- Current SQL catalog scan: 10 `control` tables and 11 functions before v4.
- Reviewed immutable migration:
  `db/migrations/0004_task_ledger_repository.sql`, 111,742 bytes, SHA-256
  `cd658ed2f4624cd0a829c818c1cf96d8ac3829264046976bdde3b2fc7feea6e5`.
- Heavy Rust and marker-owned PostgreSQL suites were deliberately not run for
  this governance-only review. Their TASK-021 evidence is compatibility
  baseline, not TASK-022 acceptance.

## Implementation Blocker And Required Re-review

The PLANS implementation blocker remains in force. It may be reconsidered only
after the responsible governance owners:

1. freeze the acyclic canonical projections/construction order;
2. freeze the retained-byte accounting algorithm;
3. add independent retained-checkpoint rollback comparison;
4. resolve vacant ordinal, singleton seeding, and first-write locking;
5. prove all nine exact scalar signatures fit the PostgreSQL input limit or
   version the function-count change;
6. correct golden-vector terminology, total transaction bounds, exact v4
   catalog totals, and the stale audit classification; and
7. obtain a fresh independent governance review of the corrected set.

ADR-020 remains proposed and must not be treated as accepted by this review.

## Residual Risks

- TASK-022 has no Rust, SQL, migration, ACL, concurrency, restart, corruption,
  or commit-uncertainty implementation evidence yet; that is expected while the
  governance blocker remains.
- The worktree is a cumulative dirty MVP-0-through-TASK-022 candidate with no
  clean per-ticket commit attribution. Exact allowlists and artifact hashes are
  documented controls, not machine-enforced isolation.
- Remote Rust/PostgreSQL CI, upstream synchronization, branch protection,
  required reviews, merge queue, and primary-branch merge authorization remain
  missing or unverified.
- Full 64-MiB global replay while holding one singleton transaction can create
  contention and memory/latency pressure even after a total timeout is defined;
  later snapshot/archive optimization requires a separately versioned design.
- Collision resistance is assumed for digest-keyed immutable observations;
  structural row equality and pure digest recomputation must still be checked
  on every load and exact reuse.
