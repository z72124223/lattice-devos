---
module_id: postgres-codebase-memory
name: LATTICE PostgreSQL Codebase Memory Adapter
version: 1.0
status: active
owner: LATTICE maintainers
last_reviewed: 2026-08-05
---

## Mission

Own the independent same-database PostgreSQL adapter, extension installation,
catalog verification, atomic persistence, exact retrieval audit, and restart
replay mechanics for Codebase Memory without entering the global Store
migration profile.

## Non-Goals

- Decide graph normalization, candidate state, ranking, retrieval meaning, or
  trust; those remain owned by `codebase-memory`.
- Modify or join Postgres Store migrations, Registry `0005`/schema-v4, the
  global migration manifest, or global Store/Ledger/Registry receipt meaning.
- Expose SQL, paths, credentials, queries, provider settings, or migration
  authority through MCP or a general runtime port.

## Owned Data

- The exact `codebase_memory` extension identity and extension ledger.
- Four normalized domain tables for complete analyses, candidate records,
  retrieval audits, and terminal receipts.
- Fixed PostgreSQL functions, catalog/ACL verification, and physical database
  transaction/replay mechanics only; semantic record/query legality remains
  owned by `codebase-memory`.

## Public Contracts

- Embed and verify one exact `db/extensions/codebase-memory/v1.sql` byte stream
  and hash; never discover SQL from a directory.
- Apply only through an explicit administrative runner after exact global-v3
  and database-identity verification; exact v1 is a verified no-op.
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

## Allowed Dependencies

- `lattice-contracts` 1.12 for typed evidence.
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

## Acceptance Gates

| Gate | Evidence | Owner | Required for merge |
|---|---|---|---|
| Embedded identity | independent exact byte/hash and typed evidence tests | Engineering | yes |
| Install/no-op/rollback | disposable PostgreSQL 17 fresh, repeat, partial/collision, and fault matrix | Engineering | yes |
| Catalog and ACL | exact schema/table/function/owner/grant verifier plus runtime direct-table denial | Security review | yes |
| Persistence/replay | exact analysis/retrieval/receipt write and process/database restart replay | Engineering | yes |
| Global compatibility | unchanged global bytes/hashes and existing Store/Ledger/Registry replay | Compatibility review | yes |
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
