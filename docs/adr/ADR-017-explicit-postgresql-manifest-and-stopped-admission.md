# ADR-017: Explicit PostgreSQL Manifest And Stopped Admission Foundation

- Status: accepted under the user's MVP-3 execution directive
- Date: 2026-08-01
- Review corrected: 2026-08-02
- Decision owner: user
- Related: SPEC-002 v21, ADR-005, ADR-016, Postgres Store 1.1.5, TASK-019

## Context

TASK-018 froze a safe physical transaction contract and non-durable fake. The
repository now needs a real PostgreSQL foundation before any durable domain
repository can exist. The current 312-byte `0001_bootstrap.sql` is an inert
draft: it contains `BEGIN`/`COMMIT`, uses `IF NOT EXISTS`, creates only three
schemas, and has no exact history, database identity, role separation, runtime
admission, or compatibility contract. Executing it inside a transaction-owning
runner would create ambiguous ownership and nested transaction behavior.

The machine has PostgreSQL 17.10 tools and an existing loopback service, but no
project database credentials or connection environment. A running installed
service is not permission, migration, restart, concurrency, or durability
evidence and must not be used as a disposable test target.

The MVP-1 runtime also does not yet contain the independent Guardian that will
own daemon activation. A migration must not invent a normal daemon path that
can promote itself to `ACTIVE`.

## Decision

Postgres Store 1.1.5 adds only an exact migration/schema compatibility foundation.
It does not implement live `ControlStore` or construct durable receipts.

### Driver and connection boundary

- Pin `postgres = 0.19.14` with default features disabled and `sha2 = 0.11.0`.
  The synchronous driver matches the existing synchronous port without an
  async bridge. SQLx, Diesel, a direct Tokio port, pools, TLS adapters, and
  libpq are out of scope.
- TASK-019 accepts a caller-supplied driver client. It does not read connection
  environment variables or retain/log a DSN or password.
- Live evidence is limited to an owned loopback-only PostgreSQL 17.10
  disposable cluster with data checksums, `fsync=on`,
  `synchronous_commit=on`, and `full_page_writes=on`.

Primary driver references:

- <https://docs.rs/postgres/0.19.14/postgres/>
- <https://www.postgresql.org/docs/17/runtime-config-connection.html>
- <https://www.postgresql.org/support/versioning/>

### Manifest and migration ownership

The manifest is a compile-time closed ordered list. Each entry fixes ordinal,
ID, path, byte length, SHA-256, status, transaction mode, schema version, and
reader/writer compatibility. Runtime directory discovery and arbitrary caller
SQL are forbidden.

The unchanged `db/migrations/0001_bootstrap.sql` and its SHA-256
`7BFF021FC17F738551309C906578C8015B2DD0307D27D239C21DF1697C4D09C8`
remain in the manifest as `SUPERSEDED`; its SQL is never executed. A new
transaction-control-free migration is the first `EXECUTABLE` entry.

Before SQL, the runner verifies an exact caller-supplied target database name,
rejects `postgres`/`template*`, and matches a pre-provisioned disposable-run
sentinel stored on that database. It then hashes every included byte and
rejects checksum drift,
unknown/missing/reordered database history, an incompatible server/schema, or
pre-existing `control`, `memory`, or `readmodel` namespaces without exact
LATTICE history. It obtains one fixed transaction-scoped PostgreSQL advisory
lock and executes all missing entries plus history rows in one runner-owned
transaction. Re-running the identical manifest is a no-op. An uncertain reply
is reconciled by reconnecting and observing exact history; changed SQL is never
retried under the same ID. A known successful commit whose post-apply verifier
fails returns `PostApplyVerificationFailed`
(`STORE_MIGRATION_COMMITTED_UNVERIFIED`); identical reconnect retry must observe
`AlreadyCurrent` and pass verification. An unknown commit outcome remains a
separate error phase.

Normal daemon startup invokes the read-only schema verifier, never the runner.

### Initial schema and authority

The executable foundation migration creates only:

- an immutable database identity;
- exact migration history and schema compatibility metadata;
- generic physical scope-head and terminal-transaction foundations for future
  Store 1.2/domain repository work;
- one runtime-admission singleton initialized to `STOPPED` with no daemon
  instance, epoch, observation, or authority head.

There is no TASK-019 command or grant that lets normal runtime elect itself or
change admission. Guardian-owned activation remains later work. Disposable
tests may inspect or deliberately corrupt fixtures through their test admin;
that is not a production API or authority.

### Roles and protected provisioning

The schema expects separate migration-owner, normal-runtime, future-Guardian,
and read-only `NOLOGIN` capability roles. The migration runner does not create
login roles, passwords, databases, or credentials. Production provisioning
remains a separate protected operation. Each capability has one fixed LOGIN
principal with exact `ADMIN FALSE, INHERIT FALSE, SET TRUE` membership. The
LOGIN receives only direct `CONNECT` on the exact target database and must
`SET ROLE` to gain its capability; before that change it has no inherited
capability, database `CREATE`/`TEMPORARY`, or direct grant in any ACL-bearing
catalog. Across the cluster, each LOGIN has exactly one non-grantable direct
target `CONNECT` granted by `lattice_migrator`, no database ACL elsewhere,
`PUBLIC` has no database ACL, and `pg_parameter_acl` grants nothing to `PUBLIC`
or any fixed capability/LOGIN role. External non-system relations and columns
have no owner or ACL among `PUBLIC` and the eight fixed roles, while external
non-system functions deny those principals effective execution. Every recorded
`pg_default_acl` owner has zero `PUBLIC` grant. Cluster-wide `pg_shdepend`
`deptype = 'o'`, cross-checked by explicit current-database owner checks, proves
that none of the four fixed LOGIN principals owns any per-database or shared
object without pretending a local catalog scan sees other databases. The
disposable cluster harness creates those
LOGIN fixtures with its one-time SCRAM secret and proves real low-privilege
authentication. Superuser `SET SESSION AUTHORIZATION` is not runtime evidence.

Normal runtime may read compatibility/admission but has no direct table DML,
schema ownership, DDL or effective CREATE on any non-system schema,
migration-history/database-identity write, admission
write, or owner/migrator role escalation. TASK-020+ may expose only narrow,
schema-qualified `SECURITY DEFINER` procedures with a fixed safe `search_path`
and no dynamic SQL; every such procedure must lock and revalidate exact daemon
instance/epoch/admission in the same transaction as its future mutation. The
Guardian role cannot write domain/physical tables. The read-only role cannot
mutate any control table. `PUBLIC` receives no schema/table/function authority,
including through default privileges.

### Review correction: snapshot, identity, and catalog closure

The migration runner uses one read-committed writable transaction under its
transaction-scoped advisory lock. This lets a waiting concurrent runner observe
the preceding runner's commit after acquiring the lock. After a successful
commit, the runner invokes the same explicit repeatable-read, read-only verifier
used at runtime. Each returned post-apply/runtime catalog proof therefore
observes one consistent snapshot.
It reads database identity, history, compatibility, and admission through
`ONLY`, signs dropped-column and relation inheritance/partition flags, rejects
any `pg_inherits` edge involving an owned relation, signs live column ACLs,
requires the exact owned table-row/generated-array `pg_type` allowlist, rejects
shell or extra types, and rejects unlisted owned-namespace object classes. The
target derives a deterministic domain-separated
SHA-256 custom UUIDv8 from the exact target name and run marker and binds that
immutable identity at first apply; a zero, malformed, random, UUIDv5-labelled,
or substituted identity fails closed.
Runtime, Guardian, reader, their login principals, and `PUBLIC` have no
effective CREATE authority in any non-system schema.

### Review correction: protected function and asynchronous-command boundary

Postgres Store 1.1.5 freezes an exact protected-function signature manifest for
the state before a fixed LOGIN principal performs `SET ROLE`:

- five large-object creators: `lo_creat(integer)`, `lo_create(oid)`,
  `lo_from_bytea(oid,bytea)`, `lo_import(text)`, and `lo_import(text,oid)`;
- both PostgreSQL 17 identity signatures
  `pg_logical_emit_message(boolean,text,text,boolean)` and
  `pg_logical_emit_message(boolean,text,bytea,boolean)`; callers may omit the
  fourth argument only because it has a default;
- exactly these sixteen advisory-lock acquisition overloads:
  `pg_advisory_lock(bigint)`, `pg_advisory_lock(integer,integer)`,
  `pg_advisory_lock_shared(bigint)`,
  `pg_advisory_lock_shared(integer,integer)`,
  `pg_advisory_xact_lock(bigint)`,
  `pg_advisory_xact_lock(integer,integer)`,
  `pg_advisory_xact_lock_shared(bigint)`,
  `pg_advisory_xact_lock_shared(integer,integer)`,
  `pg_try_advisory_lock(bigint)`,
  `pg_try_advisory_lock(integer,integer)`,
  `pg_try_advisory_lock_shared(bigint)`,
  `pg_try_advisory_lock_shared(integer,integer)`,
  `pg_try_advisory_xact_lock(bigint)`,
  `pg_try_advisory_xact_lock(integer,integer)`,
  `pg_try_advisory_xact_lock_shared(bigint)`, and
  `pg_try_advisory_xact_lock_shared(integer,integer)`;
- `pg_export_snapshot()`, `pg_current_xact_id()`, and `txid_current()`; and
- same-LOGIN `pg_cancel_backend(integer)` and
  `pg_terminate_backend(integer,bigint)` capability, proven with two real
  concurrently authenticated sessions.

Every fixed LOGIN is denied effective execution of that manifest before
`SET ROLE`. Among the sixteen lock-acquisition overloads, only
`pg_advisory_xact_lock(bigint)` is granted to `lattice_migrator`, and the grant
is available only after that LOGIN changes to the migrator capability role; it
  is never a direct LOGIN grant. Its direct ACL entry is exactly non-grantable
  and issued by the protected function owner. The server must also report
`max_prepared_transactions = 0`, eliminating prepared transactions as retained
unaccounted write authority.

`LISTEN`/`NOTIFY` is not an authoritative state, admission, evidence, or effect-
delivery channel. PostgreSQL `NOTIFY` is a SQL command rather than a function
call, so this decision explicitly does not claim that function `EXECUTE`
revocation disables it. Future code must continue to derive authority from
transactionally verified PostgreSQL rows, never an asynchronous notification.

### Disposable evidence and cleanup

The harness creates a new marked data directory below the repository test
target, selects a non-5432 loopback port, and never stops or changes the
installed `postgresql-x64-17` service/data directory. Cleanup requires the
exact resolved test root, matching ownership marker and cluster identity, and a
stopped server. Every existing ancestor is checked for a reparse point before
creation. Only `pg_ctl status` exit 3 proves stopped; inaccessible or unknown
state preserves the root and fails. PASS is emitted only after final stop,
exact cleanup, and installed-service equivalence all succeed. PostgreSQL error-
statement and bind-parameter logging is disabled for the disposable adversarial
fixture; the harness verifies retained logs contain neither its one-time secret
nor injected fixture SQL before deletion. Raw credentials, DSNs, SQL values, or
driver diagnostics are never written to logs or repository artifacts.

Required evidence includes first apply, exact no-op retry, concurrent runners,
transaction rollback, exact reconnect after an injected commit-response loss,
known-commit/post-apply-verifier failure followed by `AlreadyCurrent` retry,
unknown/missing/reordered/checksum-drifted history, pre-existing-schema/role/
controllable-setting/target-sentinel rejection, cluster database/PUBLIC/
parameter ACL drift, all-catalog LOGIN grants, external relation/column/
effective-function PUBLIC and fixed-role grants, all-owner default ACL drift,
cluster-wide fixed-LOGIN ownership drift, protected-function grants,
`max_prepared_transactions`, row/array/shell-type and column-ACL drift, post-apply
catalog/owner/identity/constraint/grant verification, runtime inability to DML/
migrate/self-activate/escalate, and stop/start persistence of identity/history/
admission. Real-login regressions prove the matching pre-`SET ROLE` denials,
including same-LOGIN backend cancellation/termination; no superuser session
impersonation substitutes for those proofs.
Major-version and non-loopback rejection remain deterministic preflight branches
but cannot both be live-produced by the one exact 17.10 loopback fixture.

## Consequences

- TASK-019 supplies a real database foundation without pretending that generic
  schema rows are domain truth or a durable Store receipt.
- Contracts 1.8, Ports 1.3, and the zero-I/O fake remain unchanged.
- TASK-020 must version Postgres Store again before implementing live physical
  transactions and durable Ledger/outbox persistence.
- Remote/TLS operation, production role/login provisioning, daemon election,
  Guardian activation, provider/product effects, release, and deployment remain
  absent.

## Rejected Alternatives

- Execute `0001` as-is: rejected because it owns transaction control and hides
  schema/owner drift with `IF NOT EXISTS`.
- Edit `0001` silently: rejected because its exact reviewed bytes/hash are
  evidence; it is preserved and explicitly superseded instead.
- Auto-discover migration files: rejected because adding a file would silently
  expand executable authority.
- Auto-migrate on normal startup: rejected because runtime must not acquire DDL
  or migration-owner authority.
- Seed `ACTIVE` or a daemon epoch: rejected because no TASK-019 owner can
  authenticate the first leader and Guardian is not implemented.
- Use the installed service/user database for tests: rejected because restart,
  role, corruption, and cleanup tests require a disposable owned boundary.
- Add SQLx for its migrator: rejected because its async/pool/macro surface is
  unnecessary for this bounded synchronous foundation.
