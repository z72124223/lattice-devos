# TASK-019 Workflow Audit

- Date: 2026-08-01
- Repository:
  `C:\Users\f7212\Documents\Codex\2026-07-29\files-mentioned-by-the-user-2026\outputs\lattice-devos-v2`
- Branch/HEAD: `feature/v2-rust-postgres-bootstrap` at
  `06c3954cd941dc6b743e9e2a1d4f94c3658b3ff9`
- Mode: read-only audit before TASK-019 implementation

## Continuity And Current Slice

TASK-018 is complete: 61 focused package tests, 380 full Rust tests, 44 Node
tests, independent code/security and architecture review, and local integration
pass. AC-32 is complete only for the zero-I/O fake. The current bounded slice
is the exact-manifest PostgreSQL 17 schema/admission foundation required before
any durable repository.

The configured project router is absent; PLANS, HANDOFF, SPEC-002, ADR-016, and
the repository path unambiguously identify LATTICE. This remains a general AI
development platform. The unrelated playmate website is not in scope.

## Repository And Local Capability Evidence

- Git remains dirty by design with MVP-0 through TASK-018 uncommitted. HEAD is
  four commits ahead and zero behind local `main`; no remote/upstream exists.
- PostgreSQL 17.10 binaries exist under
  `C:\Program Files\PostgreSQL\17\bin`; the installed service is running and
  listens only on loopback 5432. An unauthenticated `pg_isready` probe reports
  accepting connections. No SQL/login/credential/database mutation occurred.
- `initdb`, `postgres`, `pg_ctl`, `psql`, `pg_dump`, `pg_restore`, and
  `pg_basebackup` 17.10 are available, so a separate disposable cluster can be
  created without touching the installed service/data directory.
- No DATABASE/PG/LATTICE test connection environment is present. PostgreSQL
  tools are not on PATH. No repo/user Cargo config or rust-toolchain pin exists.
- Rust/Cargo 1.97.1 match the workspace minimum. No PostgreSQL driver is in
  manifests, lockfile, metadata, or local Cargo cache; adding it requires exact
  network resolution and lockfile review.
- `0001_bootstrap.sql` is 312 bytes/11 lines, untracked, unchanged at SHA-256
  `7BFF021F...C4D09C8` and blob `5c1bb6...d23ec5`. It contains its own
  transaction and only `IF NOT EXISTS` schema creation.

## Stage Classification Before Governance

| Stage/capability | Status | Evidence |
|---|---|---|
| Repository/plan/handoff | valid | TASK-018 closure and current Git state |
| Behavior specification | partial | durable goals exist; exact foundation criterion missing |
| Postgres Store constitution | stale | 1.0 forbids every TASK-019 database action |
| TASK-019 ticket/ADR | missing | no file or marker target exists |
| Driver choice | resolved | official `postgres` 0.19.14 synchronous fit; SQLx rejected |
| Explicit migration manifest | missing | only inert directory file exists |
| Transaction-owned runner | missing | `0001` is incompatible as executable input |
| Database identity/history | missing | no table or exact compatibility record |
| Runtime admission foundation | missing | spec only; no database row or permission boundary |
| First leader/ACTIVE owner | blocked/deferred | Guardian/election authority absent; bootstrap must remain STOPPED |
| Disposable database harness | possible, missing | PG17 tools exist; no owned harness yet |
| Live ControlStore/durable receipt | deferred | TASK-020+, absent by design |
| Remote CI/merge controls | missing/unverified | no remote or committed candidate |

## Blocking Findings Resolved By Governance

1. Store 1.0 forbids a driver and runner. SPEC v16, ADR-017, Store 1.1, and
   TASK-019 must activate the exact narrow exception before code.
2. `0001` cannot be safely nested in a runner transaction and `IF NOT EXISTS`
   cannot establish ownership. Preserve its bytes as `SUPERSEDED`; add a new
   executable migration.
3. Runtime must not auto-migrate, create credentials/roles, or self-promote
   admission. Separate explicit migration and read-only verification surfaces;
   initialize STOPPED/no leader.
4. The installed service is user state, not a test fixture. Use only a marked
   disposable cluster on a random loopback port with exact cleanup checks.
5. Schema/role/restart evidence does not prove a durable `ControlStore` or
   domain legality. Contracts/Ports and receipts remain unchanged.

## Required Execution Order

1. Freeze SPEC v16, ADR-017, Store 1.1, TASK-019, and one current marker.
2. Add RED unit tests for manifest closure/checksums/order/history, static
   errors, role/admission contract, and migration SQL constraints.
3. Pin/review the minimal driver graph; implement the manifest, runner, and
   read-only verifier without environment/credential loading.
4. Add the executable schema migration and a marker-owned disposable cluster
   harness; prove apply/no-op/concurrency/rollback/denials/restart/cleanup.
5. Run focused/full Rust and Node verification, strict format/Clippy,
   dependency/secret/provider/product/website scans, and diff hygiene.
6. Complete independent code/security and architecture reviews, integration,
   ledger, ticket, plan, and handoff closure before TASK-020.

No material product decision blocks this bounded local test foundation.
Production credentials/roles/database changes, public networking, remote/TLS,
daemon/Guardian activation, destructive migration, protected release, and
primary-branch merge remain separate protected actions.
