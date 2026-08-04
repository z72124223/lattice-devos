# TASK-019 Code And Security Review

## Decision

`PASS`. Final independent review found no remaining issue: P0=0, P1=0,
P2=0, P3=0, and no integration blocker.

## Reviewed Boundary

- Postgres Store 1.1.5 exact migration manifest, administrative runner, and
  read-only runtime verifier.
- PostgreSQL 17.10 database identity, history, compatibility, STOPPED
  admission, role, ownership, ACL, protected-function, and settings closure.
- Marker-owned PowerShell cluster harness, real LOGIN evidence, restart, and
  fail-closed cleanup.
- Preservation of the existing `ControlStore` fake, Contracts 1.8, Ports 1.3,
  and the explicit absence of live/durable Store receipts.

## Final Security Result

- The protected manifest contains the exact 28 PostgreSQL 17 identity
  signatures: five large-object creators, two four-argument logical-message
  emitters, sixteen advisory-lock acquisitions, two same-LOGIN backend-control
  functions, snapshot export, and two transaction-ID allocators.
- Every fixed LOGIN receives SQLSTATE `42501` for protected calls before
  `SET ROLE`. Only `lattice_migrator` receives the one exact non-grantable
  owner-issued `EXECUTE` grant on `pg_advisory_xact_lock(bigint)`.
- Cross-database fixed-LOGIN ownership closure uses `pg_shdepend` `deptype =
  'o'`; the disposable test proves detection with an object in another
  database.
- External function, relation, column, default-ACL, type, role, database,
  parameter-ACL, schema, and direct-LOGIN capability drift all fail closed.
- `max_prepared_transactions = 0` is verified and an actual
  `PREPARE TRANSACTION` attempt fails with SQLSTATE `55000`.
- `LISTEN`/`NOTIFY` remains explicitly non-authoritative; the implementation
  does not make the false claim that function ACLs disable the SQL command.
- Large-object ownership and ACL fixtures are isolated, and all diagnostic
  markers introduced during review were removed.

## Verification

- Postgres Store focused tests: 35/35.
- Full Rust workspace, all targets and features: 401/401.
- Preserved Node suite: 44/44.
- Two clean marker-owned PostgreSQL 17.10 runs report
  `TASK019_HARNESS_SELF_TEST=PASS` and `TASK019_POSTGRES_HARNESS=PASS`, including
  the initial and restart phases on a non-5432 loopback endpoint.
- Format, strict workspace Clippy, PowerShell AST parsing, dependency-tree,
  debug-marker, temporary-artifact, migration-hash, and diff-hygiene checks
  pass.

Final reviewed implementation hashes:

- `postgres_setup.rs`: `52011e859c9b3635e325b81abade2c88cafe21af6e6c771b041c24046339bd22`
- `postgres_live.rs`: `21fd65f49d17ec6a1012f4e4fffde08cd8ae375f73d92793c82aabfe9ffb27`
- `migration_contract.rs`: `2cadcbefc8097c8bbbcb6c615b5d92c76acd0fa3455c2fc7d56f2d3400b333c6`
- `run-task019-postgres.ps1`: `4be0ab988a9a39394c3c3ba48ca15129d331c9e1c0c12a4e2cfed88285d3362e`
- `0002_control_store_foundation.sql`: `e996dc64af3112a647e75ebf07df2a77b1e9b3a018ed443880150365184883f0`

## Residual Scope

Live physical `ControlStore` transactions, durable receipts, domain
repositories, daemon/Guardian activation, providers, products, and protected
release remain later tickets. No commit, push, merge, publication, deployment,
or production database/credential mutation was performed.
