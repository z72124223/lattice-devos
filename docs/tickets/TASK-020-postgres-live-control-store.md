---
ticket_id: TASK-020
spec_id: SPEC-002
spec_version: 22
module_id: postgres-store
constitution_version: 1.2
status: completed
parallel_safe: false
depends_on:
  - TASK-019
allowed_paths:
  - Cargo.toml
  - Cargo.lock
  - crates/lattice-contracts/src/lib.rs
  - crates/lattice-contracts/tests/store_contracts.rs
  - crates/lattice-ports/src/lib.rs
  - crates/lattice-ports/tests/store_port.rs
  - crates/lattice-postgres-store/Cargo.toml
  - crates/lattice-postgres-store/src/lib.rs
  - crates/lattice-postgres-store/src/live.rs
  - crates/lattice-postgres-store/src/migrations.rs
  - crates/lattice-postgres-store/src/postgres_setup.rs
  - crates/lattice-postgres-store/tests/migration_contract.rs
  - crates/lattice-postgres-store/tests/postgres_live.rs
  - crates/lattice-postgres-store/tests/postgres_store.rs
  - crates/lattice-postgres-store/tests/postgres_control_store.rs
  - db/migrations/0003_live_control_store.sql
  - scripts/run-task019-postgres.ps1
  - PLANS.md
  - HANDOFF.md
  - docs/PROJECT_CHARTER.md
  - docs/specs/SPEC-002-autonomous-development-platform.md
  - docs/adr/ADR-018-live-postgres-physical-control-store.md
  - docs/modules/V2_AMENDMENT_PROPOSAL.md
  - docs/modules/README.md
  - docs/modules/lattice-contracts/MODULE_CONSTITUTION.md
  - docs/modules/lattice-ports/MODULE_CONSTITUTION.md
  - docs/modules/postgres-store/MODULE_CONSTITUTION.md
  - docs/tickets/TASK-019-postgres-manifest-admission-foundation.md
  - docs/tickets/TASK-020-postgres-live-control-store.md
  - docs/workflow/WORKFLOW_LEDGER.md
  - docs/reviews/WORKFLOW_AUDIT_TASK_020_2026-08-02.md
  - docs/reviews/CODE_REVIEW_TASK_020_2026-08-02.md
  - docs/reviews/ARCHITECTURE_REVIEW_TASK_020_2026-08-02.md
  - docs/reviews/INTEGRATION_TASK_020_2026-08-02.md
likely_files:
  - crates/lattice-contracts/src/lib.rs
  - crates/lattice-ports/src/lib.rs
  - crates/lattice-postgres-store/src/lib.rs
  - crates/lattice-postgres-store/src/live.rs
  - crates/lattice-postgres-store/src/migrations.rs
  - crates/lattice-postgres-store/src/postgres_setup.rs
  - crates/lattice-postgres-store/tests/postgres_live.rs
  - db/migrations/0003_live_control_store.sql
branch: feature/v2-rust-postgres-bootstrap
---

## Objective

Upgrade the exact PostgreSQL foundation from schema v1 to v2 and implement one
live, function-gated, durable physical `ControlStore` over a caller-supplied
runtime client. Preserve the fake and keep every domain repository, activation,
provider, product, website, and protected release outside this ticket.

## Acceptance Criteria

- [x] Contracts 1.9 preserves Store contract v1 as fake-only, makes v2 current,
  adds exact PostgreSQL durability and persistence evidence, stores runtime/
  durability on receipts, rejects every mixed fake/live/durability/persistence
  combination, and changes no unrelated contract behavior.
- [x] Ports 1.4 changes only `ControlStore::current_head` to explicit mutable
  access and documents that physical durability never grants domain legality;
  the crate remains Contracts-only and driver-free.
- [x] `0001` and `0002` bytes/hashes remain unchanged. One exact transaction-
  control-free `0003` migration advances schema v2, adds complete terminal
  evidence fields, and creates only the three approved Store functions.
- [x] The migration runner accepts a fresh target, a verified exact/empty v1
  foundation prefix, or a full exact v2 target. It applies only missing entries
  and atomically advances history/compatibility; all drift, partial, non-prefix,
  non-empty, reordered, edited, or unknown states fail closed.
- [x] The verifier signs exact function identities, source/properties/owner/
  safe configuration/ACLs and all new columns/constraints. Runtime has exact
  EXECUTE only; PUBLIC, Guardian, reader, and fixed LOGIN roles have none.
  Runtime retains zero direct physical/terminal table SELECT or DML.
- [x] `PostgresControlStore` consumes a caller-supplied runtime client and exact
  disposable target, exposes no raw client/query, and reads/logs/retains no
  DSN, password, credential source, environment contents, raw SQL value, or raw
  driver diagnostic.
- [x] Prepare plus finalize run in one bounded SERIALIZABLE transaction.
  Replay/changed-ID classification occurs before admission. New work checks
  exact ACTIVE daemon instance/epoch/authority revision and digests plus the
  locked current physical head; finalize revalidates all mutation authority.
- [x] First scope use admits only Store-derived live genesis. Applied advances
  exactly one checked signed-BIGINT revision with head and terminal receipt
  atomic; stale stores a terminal receipt without head mutation. Scope and
  replay are isolated by project/snapshot/closed owner/aggregate digest.
- [x] A live receipt is returned only after successful commit and is exactly
  bound to Store v2, producer, Live/DurablePostgres, database identity, schema
  version/manifest, complete request, before/after heads, disposition, and
  transaction/receipt digests. Retained corruption fails closed.
- [x] Serialization/deadlock receives at most three whole pre-commit retries.
  A commit response error returns `CommitOutcomeUnknown` and no receipt; a new
  client plus exact retry returns the byte-identical retained terminal receipt.
  Changed transaction-ID reuse returns no retained receipt.
- [x] Mutable-head query returns the exact retained live head or deterministic
  live genesis without writing or exposing another scope.
- [x] Marker-owned PostgreSQL 17.10 evidence proves fresh v2 apply, exact v1 to
  v2 upgrade, no-op/concurrent runner, rollback, ACTIVE fixture denial/admission,
  apply/stale/exact retry/changed reuse, same-ID/same-scope/cross-scope
  concurrency, overflow, fault/unknown-response reconciliation, restart,
  catalog corruption, function/table ACLs, and service-safe cleanup.
- [x] The test administrator is the only ACTIVE-fixture writer. Normal runtime
  cannot activate/elect itself, migrate, obtain DDL/direct DML, call advisory
  locks directly, or use LISTEN/NOTIFY as authority.
- [x] Existing fake behavior and all prior Rust/Node tests remain passing.
  TASK-020 closes only AC-34; AC-03/04/05/19 and MVP-1 remain open.
- [x] Focused/full verification, an actual disposable PostgreSQL run,
  independent code/security and architecture review, local integration, ledger,
  and handoff all pass before TASK-020 is completed.

## Non-Goals

- Persist Task Ledger events/outbox, Registry, Writer Lease, Approval,
  Artifact, memory, projection, provider, product, or filesystem domain data.
- Create a production database/login/password/credential, generalize the
  marker-owned target, connect remotely or with TLS, or change the installed
  PostgreSQL service/user database.
- Implement daemon election, Guardian activation, self-release, service
  replacement, public networking, OpenClaw, Codex, Graphify, Hermes, or any
  companion/playmate website behavior.
- Add SQLx, Diesel, pools, direct Tokio ports, pgcrypto/other extensions,
  dynamic SQL, arbitrary row/JSON mutation, or automatic startup migration.
- Commit, push, merge, release, publish, deploy, reset, clean, or switch branch.

## Module And Constitution Constraints

- Postgres Store 1.2 owns only physical transaction/durability mechanics.
- Contracts 1.9 owns immutable receipt representation and no I/O/hash proof.
- Ports 1.4 owns only driver-free traits and errors.
- Domain modules remain the sole owners of transition legality and their
  receipts/current heads.
- One Gateway / One Truth / One Writer and project isolation remain mandatory.

## Dependencies And Overlap

`parallel_safe: false`: Contracts, Ports, Postgres Store, one shared migration
manifest, the exact verifier, and the disposable PostgreSQL harness all change
as one compatibility unit. No parallel ticket may touch those paths or schema.

## TDD Behaviors

1. RED/GREEN v1 fake compatibility and v2 fake/live receipt combination,
   persistence-substitution, and constructor matrices.
2. RED/GREEN mutable Ports signature and preserved dependency/error surface.
3. RED/GREEN three-entry manifest, exact prefix states, missing-only apply,
   v1 source proof, v2 compatibility, rollback, and no-op/concurrency.
4. RED/GREEN exact v2 table/function/ACL/source/config catalog signatures and
   all unauthorized execution/direct-table cases.
5. RED/GREEN live genesis/current-head, applied, stale, exact replay, changed
   ID, authority/admission/head/overflow, corruption, and scope isolation.
6. RED/GREEN serialization bound, commit-unknown/no-receipt, reconnect exact
   retry, restart, and response-loss reconciliation.
7. REVIEW RED/GREEN every accepted independent finding before repair.

## Verification

| Check | Expected evidence |
|---|---|
| Contracts/Ports/Store focused tests | all v1/v2 representation, trait, fake, live, retry, and corruption matrices pass |
| Disposable PostgreSQL harness | actual 17.10 fresh/upgrade/function/ACL/transaction/concurrency/restart evidence |
| Full Rust/Node | all prior behavior plus TASK-020 passes |
| Format and strict Clippy | exit 0, zero warnings |
| Cargo tree/audit | exact existing dependency graph; no new driver/domain/provider edge; audit status truthful |
| Migration scans | `0001`/`0002` unchanged; `0003` exact and transaction-control-free |
| Secret/connection scans | zero credential/DSN/environment/raw-driver retention; loopback marker target only |
| Git/diff hygiene | no conflict/whitespace errors; shared dirty baseline preserved |

## Human Gate

None for reversible code/tests and the marker-owned disposable local cluster.
Production provisioning/credentials, non-loopback exposure, destructive or
incompatible migration, real daemon/Guardian activation, security-control
changes, protected release, and primary-branch merge remain separate protected
actions and are not performed by TASK-020.
