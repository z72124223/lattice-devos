---
ticket_id: TASK-075
title: Schema-v5 Project Registry and autonomy migration reconciliation
spec_id: SPEC-002
spec_version: 34
module_id: postgres-store
constitution_version: 1.9
status: in_progress
parallel_safe: false
depends_on:
  - TASK-022
integration_sources:
  task_022_implementation: 12f71009b5baa3ff3ddd026e0912f90db6d87e56
  task_022_closure: a1aced9f5acc81e9081a966a1e953b3029e45163
  task_050_implementation: 714f3b9057db47e694adacf9aef5f37e09f31712
branch: feature/task-075-schema-v5-migration-reconciliation
implementation_worktree: lattice-worktrees/task-075-schema-v5-migration-reconciliation
implementation_base: 0f8cee695e1089d8d883d9c7647a2e105b5bcae1
implementation_head: a3599c18d9462732c3b82c9e7d302980657eeccc
allowed_paths:
  - Cargo.lock
  - crates/lattice-project-registry/src/lib.rs
  - crates/lattice-project-registry/tests/project_registry.rs
  - crates/lattice-contracts/src/graph_memory.rs
  - crates/lattice-contracts/tests/graph_memory_contracts.rs
  - crates/lattice-contracts/tests/graph_memory_normalized_contracts.rs
  - crates/lattice-postgres-store/Cargo.toml
  - crates/lattice-postgres-store/src/lib.rs
  - crates/lattice-postgres-store/src/live.rs
  - crates/lattice-postgres-store/src/migrations.rs
  - crates/lattice-postgres-store/src/postgres_setup.rs
  - crates/lattice-postgres-store/src/project_registry.rs
  - crates/lattice-postgres-store/src/task_ledger.rs
  - crates/lattice-postgres-store/tests/migration_contract.rs
  - crates/lattice-postgres-store/tests/postgres_live.rs
  - crates/lattice-postgres-store/tests/postgres_project_registry.rs
  - crates/lattice-postgres-store/tests/postgres_task_ledger.rs
  - crates/lattice-postgres-codebase-memory/src/adapter.rs
  - crates/lattice-postgres-codebase-memory/src/lib.rs
  - crates/lattice-postgres-codebase-memory/src/setup.rs
  - crates/lattice-postgres-codebase-memory/tests/adapter_api.rs
  - crates/lattice-postgres-codebase-memory/tests/extension_contract.rs
  - crates/lattice-postgres-codebase-memory/tests/postgres_live.rs
  - crates/lattice-postgres-codebase-memory/tests/setup_api.rs
  - db/migrations/0005_project_registry_repository.sql
  - db/migrations/0006_task_autonomy_receipt.sql
  - db/extensions/codebase-memory/v3.sql
  - scripts/run-task019-postgres.ps1
  - scripts/run-task050-autonomy-receipt-acceptance.ps1
  - scripts/test-task050-autonomy-receipt-acceptance.ps1
  - scripts/test-task075-schema-v5-migration-reconciliation.ps1
  - docs/specs/SPEC-002-autonomous-development-platform.md
  - docs/adr/ADR-011-task-ledger-event-receipt-and-resource-ownership.md
  - docs/adr/ADR-019-durable-postgres-task-ledger-and-outbox.md
  - docs/adr/ADR-020-durable-postgres-project-registry.md
  - docs/adr/ADR-022-exact-graphify-postgres-codebase-memory.md
  - docs/modules/lattice-contracts/MODULE_CONSTITUTION.md
  - docs/modules/postgres-store/MODULE_CONSTITUTION.md
  - docs/modules/postgres-codebase-memory/MODULE_CONSTITUTION.md
  - docs/modules/task-ledger/MODULE_CONSTITUTION.md
  - docs/tickets/TASK-050-autonomy-receipt-ledger-replay.md
  - docs/tickets/TASK-075-schema-v5-registry-autonomy-migration-reconciliation.md
  - PLANS.md
  - HANDOFF.md
removal_only_paths:
  - db/migrations/0005_task_autonomy_receipt.sql
---

# TASK-075 - Schema-v5 Registry/autonomy migration reconciliation

## Current Implementation Checkpoint

Commit `a3599c18d9462732c3b82c9e7d302980657eeccc` is the clean,
scope-reviewed TASK-075 implementation checkpoint. Its Registry/autonomy
schema-v5, Memory-v3, deterministic, and disposable acceptance evidence is
retained. TASK-076 has now completed the versioned Writer Lease v2 bridge that
allows the previously accepted `G3_M2_W1` profile to enter schema v5 without
rewriting history. This ticket is therefore `in_progress` again for its final
combined-candidate revalidation and closure under SPEC-002 v34 / Postgres Store
1.9; TASK-050 remains waiting until that closure is recorded.

## Authority And Objective

Reconcile the completed but non-ancestor TASK-022 Project Registry source with
the TASK-050 autonomy-receipt implementation without rewriting either domain's
accepted semantic history. The resulting global migration order is exactly:

1. schema v4 is the TASK-022 Project Registry migration at ordinal `0005`;
2. schema v5 is the TASK-050 autonomy-receipt expansion at ordinal `0006`.

This ticket owns only the bounded source reconciliation, schema/profile
compatibility, mixed replay, and disposable verification required to make that
order true. It does not mark TASK-050 or TASK-051 complete.

## Frozen Source And Migration Identity

- The sole accepted schema-v4 migration is the exact Git blob
  `12f7100:db/migrations/0005_project_registry_repository.sql`, 200,547 bytes,
  SHA-256
  `b7af1f8a8ac370bbfc8a5312497461587cb8a86eb32ff97e5b865c7ae9bf0dcf`.
  Its bytes, ID, ordinal, schema version, and reader/writer range are immutable.
- TASK-022 implementation commit
  `12f71009b5baa3ff3ddd026e0912f90db6d87e56` and closure commit
  `a1aced9f5acc81e9081a966a1e953b3029e45163` are narrow source/provenance
  dependencies. They are not authorization to merge or overwrite unrelated
  later work.
- TASK-050 implementation commit
  `714f3b9057db47e694adacf9aef5f37e09f31712` is the autonomy behavior source.
  Its `0005_task_autonomy_receipt.sql` ordinal is rejected as integration
  history. The autonomy expansion must be re-authored as the exact
  `db/migrations/0006_task_autonomy_receipt.sql` schema-v5 entry.
- The exact five-entry Registry-v4 global manifest commitment is
  `df3f7ca3687afaa0d1f676158725e6d2f06670e0612df7482aa9d4d244b59f0f`.
  It is deterministically derived from the TASK-022 descriptors and is the
  historical profile stored for v4 Registry command receipts.
- Migrations `0001` through `0004`, all historical Store-v2 receipts, existing
  Task Ledger events/commands/receipts/checkpoints, Registry semantic
  receipts/checkpoints, and Project Registry 1.2 pure semantics remain
  byte-identical.
- The existing Codebase Memory extension v1 and v2 SQL remain byte-identical.
  In particular, v2 is 76,866 bytes with SHA-256
  `9db54342b88f554ca76054c7a33ae72f04b412d2dfe21fae6eb4d8faf3e854e2`
  and extension-manifest commitment
  `0aedbd7d9ef7ca07fc2910d0da34c163cc83e3dd56f9b28292ae1f4f0c3c4d7e`;
  its required global schema is `3` with the exact four-entry manifest
  `09c431df18ad71a4f44239a5d2ddf6b1774b8ffec06c7f9223f0e41757f3d407`.
  These frozen values remain the identity of every receipt written under v2.
  The later five-entry
  `9378bbadf1e990e7d2617b66343b07193b2b8dd19bc8bb3dd6a3b618b134538a`
  profile contains the misplaced autonomy `0005` and is not an upgrade source.

## Fail-Closed Historical Database Classification

Before any migration DDL or migration `batch_execute`, the runner must classify
the installed migration history. A database whose ordinal `0005`, migration
ID, path, checksum, or schema compatibility represents TASK-050 autonomy
instead of the exact Registry blob above fails with the existing stable
`PostgresStoreSetupErrorKind::HistoryMismatch` /
`STORE_MIGRATION_HISTORY_MISMATCH` result.

The runner may establish its transaction, bounded preflight, and
transaction-scoped advisory lock before this classification. It must not run
DDL, rename or edit a history row, copy/drop/recreate a table, infer that the
autonomy table is safe, or automatically repair/promote such a database. The
operator must preserve it for separately authorized recovery. Partial,
unknown, edited, reordered, wrong-checksum, wrong-profile, or mixed-ordinal
histories likewise fail closed.

Accepted upgrade sources are only Fresh or the exact v1/v2/v3/v4 prefix/full
profiles defined by the manifest. The exact v4 source may advance only by
applying `0006`; exact v5 is a verified no-op. A fresh or earlier exact prefix
applies missing immutable entries in ordinal order.

## Schema-v5 Runtime And Catalog Contract

The base `control` profile after `0006` has exactly 16 tables, 47 retained
catalog functions, 19 runtime-executable functions, and 28 historical
functions retained without runtime EXECUTE. Independently versioned extension
profiles remain separately verified and do not change these base counts.

The exact v5 runtime allowlist is:

- Store: `store_prepare_v5`, `store_finalize_v5`,
  `store_current_head_v5`;
- Task Ledger: `task_ledger_prepare_v3`, `task_ledger_read_head_v3`,
  `task_ledger_read_events_v3`, `task_ledger_read_commands_v3`, and
  `task_ledger_finalize_v3`;
- Project Registry: `project_registry_prepare_v2`,
  `project_registry_read_state_v2`,
  `project_registry_read_observations_v2`,
  `project_registry_read_projects_v2`,
  `project_registry_read_commands_v2`,
  `project_registry_read_reservations_v2`,
  `project_registry_stage_command_v2`,
  `project_registry_stage_project_v2`, and
  `project_registry_finalize_v2`;
- autonomy subject persistence:
  `task_ledger_record_autonomy_receipt_v1` and
  `task_ledger_read_autonomy_receipts_v1`.

All 17 schema-v4 Store-v4, Ledger-v2, and Registry-v1 functions remain exact
catalog history but lose runtime EXECUTE. Earlier historical functions remain
ungranted. The 19 new functions retain the existing fixed-signature,
migrator-owned `SECURITY DEFINER`, schema-qualified, dynamic-SQL-free,
non-leakproof, parallel-unsafe, row-security-on, safe-search-path, bounded
timeout, zero-direct-table-access contract. No generic SQL, row, composite,
array-map, JSON authority, or alternate overload is introduced.

## Registry Receipt Profile Provenance

Schema v5 adds fixed `persistence_schema_version` and
`persistence_manifest_sha256` provenance to every retained Registry command
row and to the exact Registry v2 read/stage surface.

- `0006` backfills every existing v4 Registry command with schema version `4`
  and exact manifest commitment
  `df3f7ca3687afaa0d1f676158725e6d2f06670e0612df7482aa9d4d244b59f0f`.
- Every new schema-v5 Registry command records schema version `5` plus the
  constructor-frozen current six-entry manifest commitment observed and
  rechecked in its transaction.
- Replay reconstructs each persistence receipt from that command's retained
  profile, never from the adapter's current global profile. A v4 command read
  after v5 therefore returns the original byte-identical v4 persistence
  receipt; v5 commands use v5 evidence. Mixed v4/v5 histories must replay to
  the same Registry semantic state/checkpoint and exact per-command receipts.
- Missing, null, malformed, unsupported, substituted, v4-with-v5-profile,
  v5-with-v4-profile, or profile/history-disagreeing provenance is corruption
  and returns no Registry authority or receipt. It is never backfilled by
  normal runtime or inferred from current schema state.

These are adapter persistence facts. They do not enter Registry checkpoint,
record-set, retained-byte accounting, command-result, or authority-receipt hash
subjects and do not amend Project Registry 1.2.

## Codebase Memory Schema-v5 Compatibility

The independently governed Codebase Memory extension must advance by adding
`db/extensions/codebase-memory/v3.sql`; neither `v1.sql` nor `v2.sql` may be
edited, replaced, or reinterpreted. Lattice Contracts advances from 1.12 to
1.13 because `CodebaseMemoryPersistenceIdentity` is a public cross-module
identity: its v1/v2 constructors remain frozen to global schema `3`, while a
new v3 constructor binds extension schema `3` to global schema `5` and the
exact current global/extension manifests. The current constants may identify
v3, but they must not make a v1/v2 constructor silently use schema v5.

PostgreSQL Codebase Memory advances from 1.0 to 1.1. Its administrative runner
accepts only a fresh schema-v5 target or an exact retained v2 extension that
was installed against its frozen schema-v3 profile. It installs or advances
only to the exact v3 catalog under the existing transaction/advisory-lock,
catalog/ACL, identity, and no-startup-migration boundary. Partial, extra,
drifted, wrong-database, wrong-global-profile, or ambiguous extension state
fails closed with no success receipt; there is no downgrade or automatic
repair. A v2 extension observed beside global schema v5 is an upgrade source,
not a current runtime profile and not by itself corruption.

Every authoritative Codebase Memory analysis, retrieval, terminal graph
receipt, and reflection row retains the complete persistence profile used to
create it: global schema version/manifest and extension schema version/SQL/
manifest, with the fixed extension ID and independently verified database
identity. V3 backfills every existing authoritative row with the exact
v2/global-v3 profile above; new rows store the exact v3/global-v5 profile.
Load/replay selects and cross-checks the retained row profile before
recomputing any persistence, graph-memory, or Hermes-reflection receipt. It
never substitutes the adapter's current identity. Missing, malformed,
unsupported, cross-row/cross-profile, or digest-disagreeing provenance fails
closed.

This is physical compatibility only. It does not change Codebase Memory pure
ranking/record semantics, Graphify identity, public MCP tools or bytes, Hermes
semantics, or the base `control` catalog counts. The Memory extension remains
outside the global Store migration manifest and is counted as an extension
profile.

## Task Ledger And Autonomy Compatibility

Task Ledger 2.2 adds the closed `AUTONOMY_RECEIPT_RECORDED` event and its
event-owned fixed-scalar persistence subject while preserving every 2.1 hash
domain and historical byte. New-profile tasks require exactly one valid
autonomy event after `TASK_CREATED` and before any writable or external effect;
historical profiles replay with no synthesized event. The autonomy row, event,
command receipt, stream head/projection/checkpoint, and physical Store receipt
commit in the same fenced Ledger transaction. Exact retry remains
byte-identical; missing/duplicate/orphan/malformed/unknown-version rows fail
closed. The public four-tool/six-field MCP contract remains byte-identical.

## Acceptance Criteria

- [ ] The working tree contains the exact TASK-022 `0005` bytes and no edited
  or alternate Project Registry migration.
- [ ] The manifest is exactly six ordered entries, with Registry `0005` as
  schema v4 and autonomy `0006` as schema v5; `0001` through `0004` are
  byte-identical to their accepted sources.
- [ ] An autonomy-at-ordinal-`0005` historical database returns exact
  `STORE_MIGRATION_HISTORY_MISMATCH` before any DDL and remains unmodified.
- [ ] Fresh, exact v1/v2/v3, non-empty stopped v4, and exact v5 paths converge
  only through the immutable ordered manifest; partial/edited/reordered/
  unknown/ACTIVE sources fail closed.
- [ ] The exact base v5 catalog closure is `16 / 47 / 19 / 28`, the 19-function
  allowlist above is complete, every v4 runtime function is ungranted, and
  runtime retains zero direct protected-table SELECT/DML.
- [ ] Existing v4 Registry commands are deterministically backfilled with
  schema `4` and the exact v4 manifest commitment; new commands retain schema
  `5` and the exact v5 commitment.
- [ ] Fresh-process and PostgreSQL-restart mixed replay returns byte-identical
  v4 Registry persistence receipts, valid v5 receipts, and the identical pure
  Registry 1.2 semantic state/checkpoint.
- [ ] Registry provenance mutation, omission, cross-version substitution,
  coherent-prefix rollback, or current-profile recomputation fails closed
  without authority or automatic repair.
- [ ] Task Ledger mixed historical/autonomy replay, exact retry, fenced
  persistence, restart reconstruction, and autonomy corruption matrices pass
  without changing historical event/receipt/checkpoint bytes.
- [ ] Codebase Memory v1/v2 SQL bytes remain unchanged; the exact v2/global-v3
  identity remains constructible and historical graph/reflection receipt
  replay remains byte-identical after an exact extension-v3/global-v5 upgrade.
- [ ] Fresh schema-v5 Memory installation and exact v2-to-v3 upgrade converge
  on the same v3 catalog/ACL/identity profile. Partial, extra, drifted,
  unsupported, or profile-substituted sources fail closed with no repair or
  false receipt.
- [ ] Every existing authoritative Memory row is backfilled with exact
  v2/global-v3 profile provenance, new rows retain exact v3/global-v5
  provenance, all related rows agree, and
  missing/mutated/current-profile-substituted provenance fails closed.
- [ ] MCP discovery/input/output remains exactly four tools and the existing
  six-field `lattice.task.status.v1` result; autonomy stays internal.
- [ ] Focused Rust, disposable PostgreSQL 17, format/Clippy, repository,
  dependency, scope, and final diff gates pass with no unresolved P0/P1
  finding before either TASK-075 or TASK-050 may be completed.

Unchecked criteria are requirements, not claims that this governance pass ran
or passed them.

## TDD And Verification

1. RED an exact TASK-022 `0005` source missing from the branch and a collision
   between Registry/autonomy ordinal `0005`; GREEN the immutable Registry-v4
   prefix plus autonomy-v5 descriptor order.
2. RED a disposable autonomy-`0005` history and prove no DDL/catalog/history
   mutation; GREEN exact `HistoryMismatch` classification before apply.
3. RED v4 command replay under current-v5 profile; GREEN retained per-command
   profile provenance and byte-identical mixed replay.
4. RED missing v5 successor catalog/ACL and autonomy event-owned persistence;
   GREEN exact base counts/allowlist plus atomic Ledger/autonomy behavior.
5. RED fresh-process/MCP regressions; GREEN restart replay and unchanged public
   four-tool/six-field surface.
6. RED the current Memory v2 adapter against schema v5 and a historical v2
   receipt decoded with the current profile; GREEN exact extension-v3 upgrade,
   retained row provenance, and byte-identical v2/v3 mixed replay.

| Check | Command or service | Required evidence |
|---|---|---|
| Pure Registry | `cargo test -p lattice-project-registry --locked` | Project Registry 1.2 golden and planner/replay parity remains byte-identical |
| Migration contract | `cargo test -p lattice-postgres-store --test migration_contract --locked` | exact six-entry manifest, hashes, schema versions, base catalog/function contract, wrong-`0005` pre-DDL denial |
| Registry durability | `cargo test -p lattice-postgres-store --test postgres_project_registry --locked` | v4/v5 provenance, mixed replay, restart, corruption, reconciliation |
| Ledger/autonomy durability | `cargo test -p lattice-postgres-store --test postgres_task_ledger --locked` | atomic subject/event/receipt/checkpoint, exact retry, mixed replay |
| Memory contracts | `cargo test -p lattice-contracts --test graph_memory_contracts --test graph_memory_normalized_contracts --locked` | frozen v1/v2 global-v3 identity plus distinct current v3/global-v5 identity and substitution denial |
| Memory extension contract | `cargo test -p lattice-postgres-codebase-memory --test extension_contract --test setup_api --test adapter_api --locked` | immutable v1/v2 bytes, exact v3 embedded profile, typed API and historical-profile reconstruction |
| Memory PostgreSQL | `cargo test -p lattice-postgres-codebase-memory --test postgres_live --locked` | fresh v3, exact v2-to-v3, restart, catalog/ACL, v2/v3 receipt replay, provenance corruption denial |
| Disposable PostgreSQL | `powershell.exe -NoProfile -ExecutionPolicy Bypass -File scripts/test-task075-schema-v5-migration-reconciliation.ps1` | fresh/prefix/v4/v5, misplaced-0005 no-DDL, restart, catalog/ACL, cleanup |
| TASK-050 acceptance | `powershell.exe -NoProfile -ExecutionPolicy Bypass -File scripts/test-task050-autonomy-receipt-acceptance.ps1` | current schema-v5 fresh-process receipt replay and zero prohibited effects |
| Repository | `npm.cmd run check` and `git diff --check` | unique current ticket, document/schema checks, no whitespace error |

## Dependencies And Parallel Safety

`parallel_safe: false`. This ticket changes the sole global migration manifest,
schema compatibility classifier, Store/Ledger/Registry runtime function
profiles, Registry persistence replay, Task Ledger event persistence, Codebase
Memory identity/extension compatibility, shared PostgreSQL harness, and
migration acceptance scripts. No other work may change those paths or claim
TASK-050/TASK-051 acceptance concurrently.

TASK-050 is blocked on this ticket. TASK-051 remains blocked on TASK-050's
later clean identified schema-v5 candidate and named acceptance receipt.
TASK-022 is the sole ticket dependency. The TASK-022 and TASK-050 commits
listed in `integration_sources` are immutable source inputs, not cyclic ticket
dependencies or completed integration evidence for this worktree. Although
the current TASK-022 ticket document is paused, its implementation commit
`12f7100` and closure commit `a1aced9` are the unique completed source and
closure records that satisfy this dependency; TASK-075 does not relabel the
current TASK-022 ticket or infer any broader terminal state from it.

## Non-Goals And Authority Boundary

- No Project Registry 1.3, changed pure Registry hash/receipt/checkpoint
  semantics, rewritten v4 receipt, or migration history repair tool.
- No edit to `db/extensions/codebase-memory/v1.sql` or `v2.sql`, historical
  Memory receipt rehash, Codebase Memory pure-domain version change, or Memory
  extension entry in the global Store migration manifest.
- No schema downgrade, destructive migration, automatic repair of a misplaced
  autonomy-`0005` database, production/user database mutation, or production
  role/credential change.
- No new MCP tool/schema/field, model invocation, GitHub effect, Graphify,
  Hermes, Codebase Memory, scheduler, provider, deployment, release, or public
  exposure work.
- No changes to TASK-051, TASK-052, TASK-053, unrelated Hermes tickets/code,
  primary-branch merge, push, PR, publication, or commit under this governance
  pass.

Routine bounded implementation and marker-owned disposable PostgreSQL
verification are authorized. Any need to edit outside `allowed_paths`, change
the frozen source/hash/ordinal, accept or repair an autonomy-`0005` history,
change public MCP bytes, perform a protected action, or weaken an owner
boundary is a new decision and fails closed.
