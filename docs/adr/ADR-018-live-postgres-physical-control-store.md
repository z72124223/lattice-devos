# ADR-018: Live PostgreSQL Physical ControlStore

- Status: accepted under the user's MVP-3 execution directive
- Date: 2026-08-02
- Decision owner: user
- Related: SPEC-002 v22, ADR-005, ADR-016, ADR-017, Contracts 1.9,
  Ports 1.4, Postgres Store 1.2, TASK-020

## Context

TASK-019 created and twice verified an exact PostgreSQL 17.10 schema,
compatibility, permission, and STOPPED-admission foundation. It deliberately
left `ControlStore` live transactions unavailable: the runtime has no direct
access to `control.physical_heads` or `control.terminal_transactions`, no
owned function exists, and the only receipt constructor is fake/non-durable.

The foundation migration is immutable. Its manifest runner can apply a fresh
schema but currently treats any complete owned-schema presence as already
current; without an explicit upgrade contract it cannot safely add a third
migration. The synchronous `postgres::Client` also requires mutable access for
queries while Ports 1.3 exposes `current_head(&self)`.

TASK-020 must create the smallest real physical durability boundary without
absorbing Registry, Ledger, Lease, Approval, Artifact, Guardian, provider,
product, or release legality.

## Decision

### Shared contract and port versions

Contracts 1.9 makes Store contract version 2 current while preserving version
1 as fake-only compatibility:

- `StoreDurability` adds `DurablePostgres`.
- `StorePersistenceEvidence` binds a non-zero database-identity commitment,
  positive schema version, and non-zero manifest commitment.
- A receipt stores its runtime, durability, and optional persistence evidence.
- v1 accepts only `Fake` / `NonDurableFake` / no persistence evidence.
- v2 accepts either that fake combination or `Live` / `DurablePostgres` /
  complete persistence evidence. Crossed combinations fail construction.
- The live constructor does not itself prove currentness or durability; only
  the concrete Store may return it after verified commit/replay.

Ports 1.4 keeps the typed transaction signature and error set, but changes
`current_head` to take `&mut self`. This exposes the synchronous connection's
real mutability instead of hiding it behind interior mutability. Ports still
depends only on Contracts and exposes no driver, client, SQL, schema, or row.

### Immutable expansion migration

`0001` and `0002` remain byte-identical. A new exact
`0003_live_control_store.sql` advances the physical schema to version 2.

The runner accepts only three states:

1. a fresh marker-owned target with no owned schemas, where all exact
   executable entries run; or
2. a complete schema-v1 foundation whose database identity, STOPPED admission,
   catalog, roles, ACLs, compatibility, and migration history equal the exact
   first two manifest entries and whose physical/terminal tables are empty.
3. a complete schema-v2 target whose full manifest, catalog, roles, ACLs,
   compatibility, identity, and runtime-admission shape verify exactly; this is
   a read-only no-op path.

For state 2, database history must be an exact immutable prefix. The runner
executes only missing entries, inserts only their history rows, and updates
schema compatibility inside the same locked transaction. Unknown, duplicate,
missing, reordered, edited, non-prefix, partially upgraded, non-empty, or
catalog-drifted sources fail before migration. A full v2 target is a verified
no-op. Commit-unknown and committed-unverified meanings remain distinct.

The v2 terminal row additionally stores the Store contract version, fixed
producer, live runtime, PostgreSQL durability, database UUID, schema version,
and manifest SHA-256 so retained evidence is complete rather than inferred
from the current binary.

### Exact runtime function surface

Schema v2 creates exactly three `control` functions:

1. `control.store_prepare_v2(...)` obtains a transaction-ID-scoped advisory
   transaction lock, classifies exact replay or changed-ID reuse before
   mutable admission checks, locks/validates ACTIVE runtime admission, locks the
   exact physical scope, and returns either the retained terminal summary or
   prepared current head. A missing scope is represented by a deterministic
   virtual live genesis and is not materialized by prepare.
2. `control.store_finalize_v2(...)` rechecks replay, complete ACTIVE authority,
   the prepared current head, disposition, checked revision, all receipt
   fields, database/schema evidence, and then atomically persists either the
   advanced head plus terminal APPLIED receipt or a non-mutating terminal
   STALE receipt. Schema v2 removes the v1 terminal-to-head foreign key so a
   first-use stale receipt does not materialize or mutate a physical head.
3. `control.store_current_head_v2(...)` observes only one exact scope and never
   mutates it; a missing row is represented to Rust, which derives the canonical
   live genesis through the same Store hash domain as the fake.

The two write calls run inside one Rust-owned PostgreSQL transaction. Finalize
is independently safe if called out of sequence because it re-locks and
revalidates every mutation authority and current-head fact.

All three functions are fixed-signature, `SECURITY DEFINER`, non-leakproof,
parallel-unsafe, schema-qualified, dynamic-SQL-free, owned by
`lattice_migrator`, and configured with a safe `search_path` containing only
trusted `pg_catalog` plus explicitly qualified LATTICE relations. Creation,
revocation from `PUBLIC`, and the exact non-grantable runtime grant occur in the
runner transaction, eliminating a public-execution window. PostgreSQL's
function-security guidance requires excluding untrusted schemas from a
security-definer search path and revoking the default `PUBLIC` execution grant:
<https://www.postgresql.org/docs/17/sql-createfunction.html>.

Only `lattice_runtime` receives exact `EXECUTE`. `PUBLIC`, Guardian, reader,
all fixed LOGIN roles before `SET ROLE`, and every other fixed capability have
zero function execution. Runtime retains zero direct SELECT/INSERT/UPDATE/
DELETE on both physical tables and cannot migrate, activate itself, execute
arbitrary SQL through the Store API, or acquire advisory locks directly.

### Physical transaction and reconciliation

`PostgresControlStore` consumes a caller-supplied, already-authenticated
runtime `Client` and exact marker-owned target. It reads and stores no DSN,
password, credential source, or environment contents and exposes no client or
arbitrary-query escape hatch.

Each new mutation uses a bounded `SERIALIZABLE` transaction with
`synchronous_commit=on`, a safe search path, and row security on:

1. recompute and validate the full canonical request;
2. prepare, returning exact replay immediately when present;
3. compute the canonical applied/stale head and live receipt in Rust;
4. finalize under the still-held authority/head locks;
5. commit; only then return `Live` / `DurablePostgres` evidence.

PostgreSQL row locks remain held until transaction end and conflicting writers
wait or fail; serialization failures use SQLSTATE `40001` and deadlocks
`40P01`. Those behaviors follow PostgreSQL 17's locking and error-code
contracts: <https://www.postgresql.org/docs/17/explicit-locking.html> and
<https://www.postgresql.org/docs/17/errcodes-appendix.html>.

A waiter can establish its `SERIALIZABLE` snapshot before an advisory-lock
winner commits a first-use head or terminal row. The stale snapshot may then
surface the exact primary-key race as `23505` rather than `40001`; the adapter
treats that fixed-function-only result as the same bounded whole-attempt retry.
On the next snapshot, exact replay, changed-ID substitution, or stale-head
semantics resolves the race without exposing a generic SQL interface.

At most three full pre-commit retries are allowed for serialization/deadlock.
No retry occurs after a commit response error: that maps to
`CommitOutcomeUnknown` and returns no receipt. A fresh client plus the exact
request is the only reconciliation path; the replay row must reconstruct the
byte-identical receipt even when admission, epoch, or physical head changed.
Changed transaction-ID reuse returns a static substitution error without
receipt disclosure.

### Activation and evidence boundary

Migration remains STOPPED/no leader. The disposable test administrator may
install an exact ACTIVE fixture solely to prove the live Store. No runtime
function, constructor, API, grant, or migration lets a normal daemon activate,
elect, or promote itself. Production target/provisioning, remote/TLS operation,
credentials, Guardian activation, and service replacement remain later
protected work.

TASK-020 closes only the new physical Store criterion AC-34. AC-03, AC-04,
AC-05, and AC-19 remain open for their domain repositories, outbox/filesystem
effects, and end-to-end restart evidence.

## Consequences

- PostgreSQL becomes real durable physical transaction evidence without a
  second truth source or a domain-semantic repository.
- The fake remains an I/O-free conformance oracle and cannot claim durability.
- The migration runner gains an exact expansion path rather than silently
  assuming every existing schema is current.
- The concrete adapter owns connection mutation and error mapping; Ports stays
  driver-free.
- TASK-021 through TASK-024 can compose domain-owned plans/receipts into this
  Store without granting those domains raw SQL.

## Implementation And Verification Status

Implemented and locally verified on 2026-08-02. The accepted design is now
represented by Contracts 1.9, Ports 1.4, Postgres Store 1.2,
`0003_live_control_store.sql`, and `PostgresControlStore`.

Direct evidence includes 409 Rust tests, 44 preserved Node tests, strict
format/Clippy, a clean `cargo audit` over 109 locked dependencies, and one
marker-owned PostgreSQL 17.10 initial/restart harness run covering fresh and
v1-prefix upgrade, apply/stale/replay/substitution, concurrency, bounded
serialization failure, signed-`BIGINT` overflow, retained-row corruption, and
commit-response-loss reconciliation. Independent code/security and
architecture reviews report zero remaining P0 through P3 findings; local
combined integration passes.

This implementation status closes only SPEC-002 AC-34. It does not convert the
physical Store into a Task Ledger, Registry, Lease, Approval, Artifact,
Guardian, provider, product, release, deployment, or production-database
authority.

## Rejected Alternatives

- Edit `0002`: rejected because its reviewed bytes and checksum are immutable
  migration evidence.
- Grant runtime direct table DML or SELECT: rejected because it enables
  arbitrary cross-project rows and bypasses same-transaction authority checks.
- One generic JSON/SQL execution function: rejected because it creates an
  unbounded row/command surface and a second canonicalization implementation.
- Compute canonical receipt hashes in PL/pgSQL: rejected because Rust/cjson
  already owns those subjects and duplicate implementations can drift.
- Hide mutable `Client` behind `RefCell` or `Mutex` solely to preserve the old
  trait: rejected because the query's mutation should remain explicit.
- Return a receipt before commit or retry a commit error: rejected because an
  unknown outcome is neither failure nor durable success.
- Add Ledger/outbox persistence now: rejected because it would combine the
  physical Store with a separate domain owner and make TASK-020 unbounded.
