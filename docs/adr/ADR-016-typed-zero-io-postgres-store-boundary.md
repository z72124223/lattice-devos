# ADR-016: Typed Zero-I/O Postgres Store Boundary

- Status: accepted under the user's MVP-3 execution directive
- Date: 2026-08-01
- Decision owner: user
- Related: SPEC-002 v15, ADR-004, ADR-005, ADR-008, TASK-018

## Context

ADR-005 makes PostgreSQL the only durable truth and assigns physical tables,
transactions, indexes, roles, and migrations to `postgres-store`. The initial
Rust port, however, is only
`append(AppendCommand) -> ControlStoreEvidence`. It cannot bind a canonical
project and snapshot, a closed repository owner, an independently retained
daemon authority head, an expected physical head, exact transaction retry, or
an unknown commit outcome. Generic evidence also risks making an in-memory
fake look like a durable PostgreSQL commit.

The domain modules already own their legal transitions and complete semantic
receipts. Project Registry owns project lifecycle, Task Ledger owns event and
resource semantics, Writer Lease owns fencing/recovery, Approval Verifier owns
challenge/proof/claim semantics, and Artifact Store owns object/reference/
quota/sweep semantics. A persistence adapter must not become another domain
owner or a generic database-write escape hatch.

## Decision

Postgres Store 1.0 first activates as a typed, bounded, zero-I/O physical
transaction boundary and deterministic in-memory conformance fake.

The shared request contains only:

- one globally unique bounded transaction ID;
- one canonical `ProjectId` plus immutable `ProjectSnapshotId`;
- one closed repository owner and opaque non-zero aggregate-key digest;
- one domain-command digest retained as a commitment, not reinterpreted;
- one complete expected daemon authority head;
- one complete expected physical compare-and-swap head;
- non-zero commitments to the canonical record set, next state, and terminal
  domain receipt, plus optional checkpoint and outbox commitments.

There is no SQL, schema, table, column, path, arbitrary key/value payload,
domain-success Boolean, provider request, or protected-action field.

`lattice-postgres-store` recomputes a domain-separated request digest from all
request fields. A transaction ID is globally unique inside the store. Exact
retry with the same recomputed digest returns the identical terminal receipt
before checking mutable authority or head state. Reusing the ID with any
changed field is a permanent substitution rejection with no state change and
no receipt disclosure.

For a new transaction the fake checks, in order:

1. structural bounds and request digest construction;
2. exact equality with its independently retained current daemon authority;
3. `ACTIVE` normal-runtime admission;
4. exact equality with the independently retained physical head;
5. checked non-wrapping revision increment and bounded capacity;
6. one all-or-none update of the physical head and terminal replay receipt.

A stale physical head may produce a terminal non-mutating denial receipt so an
exact retry remains stable. Authority/admission/capacity/corruption failures do
not create an authorized mutation or terminal success. An injected failure
before apply changes nothing. An injected response loss after apply returns
`OUTCOME_UNKNOWN`; retrying the exact request converges to the already retained
terminal receipt.

Every TASK-018 receipt is fixed to the Postgres Store producer/version,
`RuntimeKind::Fake`, and `NonDurableFake`. It proves only deterministic fake
acceptance of the physical transaction contract. It cannot prove PostgreSQL
commit, durability, restart survival, database time, isolation level, role
enforcement, daemon leadership, runtime admission enforcement in a database,
or domain legality.

## Physical Versus Domain Authority

A Store physical head is only a compare-and-swap token for one closed physical
aggregate address. It is not a Project Registry, Task Ledger, Writer Lease,
Approval Verifier, Artifact Store, Policy, Review Runtime, or Guardian current
head. A Store receipt is not approval, lease, effect, task, release, or domain
authority.

Physical transaction idempotency supplements but never replaces each domain
owner's command/idempotency rules. Later repository adapters must first consume
the domain owner's public planner/verifier and complete receipt/current-head
contracts, then map that approved commitment into this physical transaction.
`postgres-store` may verify equality and atomicity but may not invent or relax
the domain transition.

## Migration Ownership

Postgres Store owns the future explicit migration manifest, ordered migration
IDs, checksums, schema contract, and reader/writer compatibility range. A
migration runner must execute only entries named by that manifest; it may not
auto-discover and execute every file in a directory.

TASK-018 does not define or execute a migration runner and does not read or
modify `db/migrations/0001_bootstrap.sql`. TASK-019 must explicitly adopt or
supersede that inert draft, add an exact-version driver, and prove checksums,
locking, roles, restart, version compatibility, runtime admission, and
disposable-database behavior before any durable claim.

## Dependency Direction

```text
lattice-postgres-store -> lattice-ports -> lattice-contracts
                      \-> lattice-cjson
```

TASK-018 has no domain-crate or PostgreSQL-driver dependency. Domain modules do
not depend on Store. Later repository adapters may consume exactly one domain
owner's public planner/verifier without creating adapter-to-adapter calls or a
reverse dependency into that domain.

## Consequences

- The stale nominal append port is replaced by a complete typed transaction
  and physical-head query.
- Tests can prove atomic fake behavior and reconciliation semantics before
  database installation or connection.
- TASK-018 cannot close the durable portions of AC-03, AC-04, AC-05, AC-19, or
  the MVP-1 PostgreSQL exit gate.
- TASK-019 through TASK-025 remain responsible for migration/runtime admission
  and the individual durable repositories/filesystem adapter.
- Any future live receipt or migration execution requires a versioned Postgres
  Store amendment and real disposable PostgreSQL evidence.

## Rejected Alternatives

- Keep `AppendCommand`: rejected because it cannot bind physical transaction
  safety or unknown commit reconciliation.
- Accept generic SQL or row maps: rejected because this creates an arbitrary
  write path and lets Store invent domain state.
- Reuse generic `ControlStoreEvidence`: rejected because component/runtime
  labels do not prove commit certainty or durability.
- Treat the fake map as database truth: rejected because it has no restart,
  role, isolation, clock, migration, or durability evidence.
- Run the existing migration draft automatically: rejected because filenames
  are not an approved checksum manifest.

## Later Narrow Amendment

ADR-020 narrows only the statement that every future domain repository must
map into project-scoped `StoreScope`. Project Registry is a global aggregate:
registration denial may have no authority snapshot and cross-project identity
reservations require one global order/checkpoint. TASK-022 therefore uses a
separately versioned Registry-specific global transaction and persistence
receipt. It does not make `StoreScope` optional, change Store-v2 receipts, or
authorize another domain/global exception without its own constitution and ADR.
