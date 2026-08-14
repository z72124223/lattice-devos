---
module_id: postgres-codebase-memory
name: LATTICE PostgreSQL Codebase Memory Adapter
version: 1.1
status: active
owner: LATTICE maintainers
last_reviewed: 2026-08-14
---

## Mission

Own the independent same-database PostgreSQL adapter, extension installation,
catalog verification, atomic persistence, exact retrieval audit, and restart
replay mechanics for Codebase Memory without entering the global Store
migration profile.
Version 1.1 adds only the extension-v3/global-v5 compatibility profile and
historical per-analysis persistence provenance.

## Non-Goals

- Decide graph normalization, candidate state, ranking, retrieval meaning, or
  trust; those remain owned by `codebase-memory`.
- Modify or join Postgres Store migrations, Registry `0005`/schema-v4, the
  global migration manifest, or global Store/Ledger/Registry receipt meaning.
- Edit extension v1/v2 bytes, rehash a historical receipt with the current
  profile, downgrade or automatically repair extension history.
- Expose SQL, paths, credentials, queries, provider settings, or migration
  authority through MCP or a general runtime port.

## Owned Data

- The exact `codebase_memory` extension identity and extension ledger.
- Four normalized domain tables for complete analyses, candidate records,
  retrieval audits, and terminal receipts.
- Fixed PostgreSQL functions, catalog/ACL verification, and physical database
  transaction/replay mechanics only; semantic record/query legality remains
  owned by `codebase-memory`.
- Complete retained global/extension persistence profile provenance for every
  authoritative analysis, retrieval, terminal graph-receipt, and reflection
  row.

## Public Contracts

- Embed and verify explicit immutable extension v1/v2 bytes plus the new exact
  `db/extensions/codebase-memory/v3.sql`; never discover SQL from a directory.
- Apply only through an explicit administrative runner after exact global-v5
  and database-identity verification. Fresh v3 and exact v2-to-v3 are the only
  successful v3 paths; exact v3 is a verified no-op.
- Implement `CodebaseMemoryPort` using only typed analysis and retrieval-plan
  values plus typed database/extension identity evidence.
- Persist one complete analysis/candidate set atomically, audit one exact
  deterministic retrieval atomically, and replay only the exact terminal
  project/commit/query receipt after restart.
- Give normal runtime fixed `SECURITY DEFINER` execution only and zero direct
  table access.

## Invariants

1. PostgreSQL remains the one durable truth; the extension is not an alternate
   database, migration history, writer, or authority source.
2. Global Store v3, Store/Ledger receipts, Registry-reserved `0005`/schema-v4,
   and the global manifest remain byte-identical.
3. Partial, colliding, substituted, corrupt, cross-project, cross-commit, or
   outcome-unknown state never becomes a success receipt.
4. The adapter never computes or changes Codebase Memory domain legality.
5. Runtime receives no direct table privilege and cannot install or alter the
   extension.
6. Extension v1/v2 SQL bytes and their global-v3 persistence identities remain
   immutable; v3/global-v5 is a distinct identity, never an alias.
7. V3 backfills every old authoritative row with its exact v2 profile and
   records v3 for every new authoritative row. Related rows must agree, and
   replay chooses the retained row profile before computing any persistence,
   graph-memory, or reflection receipt.
8. Missing, malformed, unsupported, substituted, or digest-disagreeing profile
   provenance fails closed; the current adapter identity is not a fallback.

## Allowed Dependencies

- `lattice-contracts` 1.13 for typed versioned persistence evidence.
- `lattice-ports` 1.8 for the repository boundary.
- `lattice-codebase-memory` 1.1 for pure verification/plans.
- `lattice-cjson` for canonical digest mechanics.
- Exact synchronous PostgreSQL and SHA-256 crates used by the existing locked
  workspace.

## Forbidden Dependencies

- `lattice-postgres-store`, Task Ledger, Project Registry, Orchestrator,
  Graphify/Codex/Hermes adapters, policy, approval, Guardian, provider/model
  SDKs, dynamic SQL/migration discovery, environment/credential loaders, or
  product repositories.
- Any adapter-to-adapter call or reverse dependency from a semantic owner.

## Failure, Compatibility, And Migration

Version 1.0 accepts only exact global schema v3 plus exact Memory extension v1.
Fresh install and exact no-op are the only successful administrative states.
Partial/colliding/drifted profiles fail with zero committed extension change;
runtime corruption or ambiguity fails closed and requires exact replay or
reconciliation. No downgrade, global migration mutation, or automatic startup
migration exists.

Version 1.1 preserves extension v1/v2 SQL and every receipt created under the
v2/global-v3 identity. It adds extension v3 bound to global schema v5, exact
fresh and v2-upgrade paths, and per-analysis profile provenance for
byte-identical v2/v3 mixed replay. The extension remains outside the global
manifest. Partial/drifted/extra/ambiguous state and provenance corruption fail
closed without repair, downgrade, or false success.

## Acceptance Gates

| Gate | Evidence | Owner | Required for merge |
|---|---|---|---|
| Embedded identity | independent exact byte/hash and typed evidence tests | Engineering | yes |
| Install/no-op/rollback | disposable PostgreSQL 17 fresh, repeat, partial/collision, and fault matrix | Engineering | yes |
| Catalog and ACL | exact schema/table/function/owner/grant verifier plus runtime direct-table denial | Security review | yes |
| Persistence/replay | exact analysis/retrieval/receipt write and process/database restart replay | Engineering | yes |
| Global compatibility | unchanged global bytes/hashes and existing Store/Ledger/Registry replay | Compatibility review | yes |
| V3 profile transition | immutable v1/v2 hashes, fresh-v3/exact-v2-upgrade/no-op, and partial/drift/extra rollback matrices | Integration review | yes |
| Historical receipt identity | v2 provenance backfill, v2/v3 mixed graph/reflection replay, and missing/substituted/current-profile denial | Compatibility review | yes |
| Dependency direction | Cargo metadata and forbidden-dependency scan | Architecture review | yes |

## Change Policy

Mission, ownership, SQL/profile identity, table/function surface, transaction or
replay semantics, dependencies, or privilege model require a versioned
constitution amendment, SPEC/ADR trace, architecture review, and responsible
user approval.

## Amendment History

| Version | Date | Decision reference | Summary | Approver |
|---|---|---|---|---|
| 1.0 | 2026-08-05 | SPEC-002 v28, ADR-022, TASK-033 | Independent same-database Codebase Memory extension and adapter without global Store migration changes | User continuation authorization |
| 1.1 | 2026-08-14 | SPEC-002 v32, ADR-022, TASK-075 | Add extension-v3/global-v5 compatibility and retained per-analysis profile replay while preserving v1/v2 bytes and receipts | User-approved TASK-075 reconciliation |
