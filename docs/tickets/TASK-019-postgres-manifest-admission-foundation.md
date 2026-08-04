---
ticket_id: TASK-019
spec_id: SPEC-002
spec_version: 21
module_id: postgres-store
constitution_version: 1.1.5
status: completed
parallel_safe: false
depends_on:
  - TASK-018
allowed_paths:
  - Cargo.toml
  - Cargo.lock
  - crates/lattice-postgres-store/**
  - db/migrations/0002_control_store_foundation.sql
  - scripts/run-task019-postgres.ps1
  - PLANS.md
  - HANDOFF.md
  - docs/PROJECT_CHARTER.md
  - docs/specs/SPEC-002-autonomous-development-platform.md
  - docs/adr/ADR-017-explicit-postgresql-manifest-and-stopped-admission.md
  - docs/modules/V2_AMENDMENT_PROPOSAL.md
  - docs/modules/README.md
  - docs/modules/postgres-store/MODULE_CONSTITUTION.md
  - docs/tickets/TASK-019-postgres-manifest-admission-foundation.md
  - docs/workflow/WORKFLOW_LEDGER.md
  - docs/reviews/WORKFLOW_AUDIT_TASK_019_2026-08-01.md
  - docs/reviews/CODE_REVIEW_TASK_019_2026-08-01.md
  - docs/reviews/ARCHITECTURE_REVIEW_TASK_019_2026-08-01.md
  - docs/reviews/INTEGRATION_TASK_019_2026-08-01.md
likely_files:
  - Cargo.lock
  - crates/lattice-postgres-store/Cargo.toml
  - crates/lattice-postgres-store/src/lib.rs
  - crates/lattice-postgres-store/src/migrations.rs
  - crates/lattice-postgres-store/tests/migration_contract.rs
  - crates/lattice-postgres-store/tests/postgres_live.rs
  - db/migrations/0002_control_store_foundation.sql
  - scripts/run-task019-postgres.ps1
branch: feature/v2-rust-postgres-bootstrap
---

## Objective

Add the exact-manifest PostgreSQL 17 schema, compatibility, role, and STOPPED-
admission foundation using a marker-owned disposable cluster. Keep the existing
fake, Contracts, Ports, and receipts unchanged; do not implement live
`ControlStore` or a durable domain repository.

## Acceptance Criteria

- [x] Pin `postgres = 0.19.14` with default features disabled and
  `sha2 = 0.11.0`; review the complete new Cargo dependency graph and retain
  Rust 1.97 compatibility.
- [x] Expose one compile-time ordered manifest whose entries bind ordinal, ID,
  path, byte length, exact SHA-256, status, transaction mode, schema version,
  and reader/writer compatibility.
  No runtime glob/directory discovery or arbitrary caller migration is exposed.
- [x] Preserve `0001_bootstrap.sql` byte-for-byte at its TASK-018 hash and mark
  it `SUPERSEDED`/non-executable. Add one transaction-control-free executable
  migration; previously reviewed migration bytes are never silently edited.
- [x] Reject included-byte checksum drift before SQL; reject unknown, missing,
  duplicate, reordered, incompatible, or checksum-drifted database history.
- [x] Before the first SQL mutation, require the exact non-default target
  database name and a matching pre-provisioned disposable-run sentinel; reject
  `postgres`, `template*`, a wrong target, or a missing/changed sentinel.
- [x] Reject a database where any owned schema pre-exists without exact LATTICE
  migration history/database identity; never adopt an ambiguous namespace.
- [x] Apply all missing executable entries and their exact history rows inside
  one runner-owned transaction under a fixed transaction-scoped advisory lock.
  Exact rerun is a no-op; concurrent runners converge to one applied history.
- [x] Create only database identity, compatibility/history, generic future
  physical-head/terminal-transaction foundations, and one runtime-admission
  singleton initialized to `STOPPED` with no leader/epoch/authority digest.
- [x] Migration owner, normal runtime, future Guardian, and read-only capability roles are
  permission-separated. The runner creates no login/password/database/real
  credential. Four externally provisioned fixed LOGIN principals each have one
  exact `ADMIN FALSE, INHERIT FALSE, SET TRUE` membership and must `SET ROLE`
  to the matching NOLOGIN capability. Each LOGIN has only a direct `CONNECT`
  grant on the exact target database before `SET ROLE`, with no inherited
  capability or other database/schema/object authority. Cluster-wide closure
  requires `PUBLIC` database ACL zero, exactly four non-grantable target LOGIN
  `CONNECT` grants from `lattice_migrator` and none elsewhere, and no
  `pg_parameter_acl` grant to `PUBLIC` or any of the eight fixed roles. LOGIN
  direct grants are closed across every PostgreSQL ACL-bearing catalog;
  external non-system relation/column ownership and ACLs are closed for
  `PUBLIC` plus all eight roles, and external non-system functions deny those
  principals effective execution. Every recorded `pg_default_acl` owner has
  zero `PUBLIC` grant. Cluster-wide `pg_shdepend` `deptype = 'o'`, cross-checked
  by explicit current-database owner checks, closes ownership by any fixed
  LOGIN. Before `SET ROLE`,
  an exact protected-function manifest denies the four fixed LOGIN principals
  all five large-object creators, both four-argument `pg_logical_emit_message` overloads,
  all sixteen advisory-lock acquisition overloads, `pg_export_snapshot()`,
  `pg_current_xact_id()`, and `txid_current()`. Two real sessions authenticated
  as the same fixed LOGIN prove denial of `pg_cancel_backend(integer)` and
  `pg_terminate_backend(integer,bigint)`. Only
  `pg_advisory_xact_lock(bigint)` is granted to `lattice_migrator` after
  `SET ROLE`; it is never a direct LOGIN grant, and its allowed capability-role
  ACL is non-grantable and issued by the function owner. The disposable harness
  proves pre-`SET ROLE` denial plus real authentication using one-time fixture
  credentials and never substitutes superuser session impersonation.
  Runtime has no direct table DML, DDL, schema ownership, effective CREATE in
  any non-system schema, history/
  identity/admission write, migration, role escalation, or self-promotion;
  Guardian cannot write physical/domain tables; reader is read-only; `PUBLIC`
  has no owned schema/table/function or default privilege.
- [x] Normal startup uses a read-only verifier and never auto-migrates. It
  validates PostgreSQL major 17, UTF-8/required durability settings/data
  checksums, `max_prepared_transactions = 0`, loopback connection, exact
  manifest/identity/compatibility,
  STOPPED bootstrap, and effective runtime role.
- [x] Migration uses one explicit read-committed writable transaction under its
  transaction-scoped advisory lock so a waiting runner observes the preceding
  commit. After a successful commit it invokes the same explicit repeatable-
  read/read-only verifier used by normal startup; every returned catalog proof
  therefore uses one consistent snapshot. A known commit followed by verifier
  failure is `PostApplyVerificationFailed`
  (`STORE_MIGRATION_COMMITTED_UNVERIFIED`); identical reconnect retry must
  return `AlreadyCurrent` and pass verification, while unknown commit outcome
  remains distinct. Verification reads fixed truth tables with `ONLY`,
  rejects inheritance/partition and dropped-column tombstones, includes live
  column ACLs, requires the exact owned row/array `pg_type` allowlist, rejects
  shell/extra types, and checks the actual PostgreSQL catalog, owners,
  constraints, grants, and default privileges after apply and at runtime.
  Database identity is a deterministic domain-separated SHA-256 custom UUIDv8
  bound to the exact
  target and run marker; an exact history
  row cannot authorize a drifted catalog. TASK-019 creates no writable
  `SECURITY DEFINER` procedure; future procedures must be narrow,
  schema-qualified, safe-search-path, dynamic-SQL-free, and same-transaction
  authority/admission checked.
- [x] `LISTEN`/`NOTIFY` is not an authoritative state, admission, evidence, or
  effect-delivery source. Because `NOTIFY` is a SQL command rather than a
  function call, TASK-019 does not claim that function revocation closes it;
  all authority continues to come from verified transactional rows.
- [x] All library errors use bounded static codes and redacted Debug/Display;
  no DSN, password, environment contents, SQL value, server path, or raw driver
  diagnostic is retained or emitted.
- [x] A PowerShell harness rejects every existing reparse-point ancestor before
  creation and creates only a new marker-owned PostgreSQL 17.10
  cluster below the repository test target on a non-5432 loopback port. It
  never contacts/stops/reconfigures the installed service/data root and
  deletes only the exact marked test root after `pg_ctl status` proves exit 3.
  Unknown/inaccessible status preserves the root and fails; PASS is emitted
  only after final stop, cleanup, and installed-service comparison succeed.
- [x] Disposable integration proves first apply, exact no-op retry, concurrent
  runners, partial-failure rollback, altered manifest/history/order/namespace/
  target-sentinel/catalog/owner/constraint/grant/role/settings/server denial,
  uncertain-response and committed-unverified reconciliation, cluster database/
  PUBLIC/parameter ACL closure, all-catalog LOGIN and external relation/column/
  effective-function ACL denial, all-owner default-ACL closure, cluster-wide
  fixed-LOGIN ownership closure, exact protected-function and same-LOGIN
  backend-control denial, zero prepared transactions, exact owned row/array
  type plus shell-type denial, runtime
  inability to DML/migrate/self-activate/escalate, and stop/start persistence of
  exact identity/history/admission.
- [x] `ControlStore`, Contracts 1.8, Ports 1.3, fake receipt behavior, and all
  prior tests remain unchanged. TASK-019 emits no live/durable Store receipt
  and does not claim AC-03/04/05/19 complete.
- [x] AC-33 closes only after focused/full local checks, an actual disposable
  PostgreSQL run, independent code/security and architecture reviews, and
  local integration evidence all pass.

## Non-Goals

- Implement live physical transactions, durable Ledger/outbox or another
  domain repository, pool connections, or return `RuntimeKind::Live` receipts.
- Create/elect/activate a daemon or Guardian, modify the installed PostgreSQL
  service/user database, provision production roles/logins/passwords, or use a
  real credential.
- Add remote networking, TLS/cloud database support, SQLx, Diesel, a direct
  async port, arbitrary SQL, directory discovery, or automatic migrations.
- Call OpenClaw, Codex, Graphify, Hermes, a provider, product repository,
  payment, publication, deployment, or unrelated companion/playmate website.
- Commit, push, merge, release, deploy, or change primary branch.

## TDD Behaviors

1. RED/GREEN manifest exact fields/order/status/compatibility and full checksum
   matrix, including the superseded fixed `0001`.
2. RED/GREEN target name/sentinel, clean apply, history insertion, exact no-op,
   pre-existing schema,
   unknown/missing/reordered/tampered history, and transaction rollback.
3. RED/GREEN concurrent advisory-lock runners, uncertain-response exact-history
   reconciliation, and known-commit/post-verifier-failure `AlreadyCurrent`
   reconciliation.
4. RED/GREEN database identity, catalog/owner/constraint/grant/default-
   privilege signature, cluster database/PUBLIC/parameter ACLs, all ACL-bearing
   LOGIN grants, external relation/column/effective-function PUBLIC and fixed-
   role grants, all-recorded-owner default ACLs, cluster-wide `pg_shdepend`
   plus explicit local fixed-LOGIN ownership, exact row/array type closure and shell-type
   denial, generic table constraints, STOPPED/no-leader bootstrap, and complete
   role permission/escalation matrix.
5. RED/GREEN exact protected-function manifest with real pre-`SET ROLE` LOGIN
   calls: 5 large-object creators, 2 four-argument logical-message emitters, 16 advisory-lock
   acquisitions, same-LOGIN cancel/terminate, snapshot export, and both
   transaction-ID allocators; only post-`SET ROLE` migrator
   `pg_advisory_xact_lock(bigint)` succeeds. Require
   `max_prepared_transactions = 0` and prove notifications are ignored rather
   than claiming function ACLs deny the `NOTIFY` command.
6. RED/GREEN read-only verifier for version/settings/loopback/manifest/role,
   including runtime denial of DDL/migration/admission mutation.
7. RED/GREEN redacted static errors and zero secret/DSN/environment retention.
8. RED/GREEN harness ownership/path/port/service separation/restart/cleanup.
9. REVIEW RED/GREEN every accepted independent finding before repair.

## Verification

| Check | Expected evidence |
|---|---|
| Store focused tests | manifest/runner/verifier/error-phase/ACL/type matrices pass |
| Disposable PostgreSQL harness | actual 17.10 apply/concurrency/permission/ACL/type/retry/restart evidence |
| Full Rust/Node | all prior behavior plus TASK-019 passes |
| Format and strict Clippy | exit 0, zero warnings |
| Cargo tree/audit | exact minimal driver graph; no SQLx/TLS/domain/provider edge |
| Migration scans | 0001 hash unchanged; 0002 exact manifest-only and no transaction control |
| Secret/connection scans | no credential/DSN persistence; live endpoint loopback only |
| Git/diff hygiene | no conflict/whitespace errors; shared dirty baseline preserved |

## Human Gate

None for the marker-owned disposable local cluster and reversible code/tests.
Production database/role/login/credential changes, non-loopback exposure,
destructive/incompatible migration, daemon/Guardian activation, security-
control changes, protected release, and primary-branch merge remain separately
protected and are not performed by TASK-019.
