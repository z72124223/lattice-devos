---
ticket_id: TASK-018
spec_id: SPEC-002
spec_version: 15
module_id: postgres-store
constitution_version: 1.0
status: completed
parallel_safe: false
depends_on:
  - TASK-017
allowed_paths:
  - Cargo.toml
  - Cargo.lock
  - crates/lattice-contracts/**
  - crates/lattice-ports/**
  - crates/lattice-postgres-store/**
  - scripts/check-project.mjs
  - test/project-governance-check.test.js
  - PLANS.md
  - HANDOFF.md
  - docs/PROJECT_CHARTER.md
  - docs/specs/SPEC-002-autonomous-development-platform.md
  - docs/adr/ADR-016-typed-zero-io-postgres-store-boundary.md
  - docs/modules/V2_AMENDMENT_PROPOSAL.md
  - docs/modules/README.md
  - docs/modules/lattice-contracts/**
  - docs/modules/lattice-ports/**
  - docs/modules/postgres-store/**
  - docs/tickets/TASK-018-postgres-store-boundary.md
  - docs/workflow/WORKFLOW_LEDGER.md
  - docs/reviews/WORKFLOW_AUDIT_TASK_018_2026-08-01.md
  - docs/reviews/CODE_REVIEW_TASK_018_2026-08-01.md
  - docs/reviews/ARCHITECTURE_REVIEW_TASK_018_2026-08-01.md
  - docs/reviews/INTEGRATION_TASK_018_2026-08-01.md
likely_files:
  - Cargo.toml
  - Cargo.lock
  - crates/lattice-contracts/src/lib.rs
  - crates/lattice-contracts/tests/contracts.rs
  - crates/lattice-ports/src/lib.rs
  - crates/lattice-ports/tests/ports.rs
  - crates/lattice-postgres-store/Cargo.toml
  - crates/lattice-postgres-store/src/lib.rs
  - crates/lattice-postgres-store/tests/postgres_store.rs
  - scripts/check-project.mjs
  - test/project-governance-check.test.js
branch: feature/v2-rust-postgres-bootstrap
---

## Objective

Replace the nominal control-store append boundary with a complete typed,
project-scoped physical transaction contract and deterministic zero-I/O fake.
Prove authority/current-head comparison, all-or-none fake application, exact
transaction retry, substitution denial, unknown-outcome recovery, and explicit
non-durability before any PostgreSQL connection or migration execution.

## Acceptance Criteria

- [x] Contracts 1.8 provides bounded transaction/daemon identifiers, closed
  repository owner, exact project/snapshot/aggregate scope, independently
  comparable daemon authority and physical heads, request commitments,
  terminal receipt/disposition, and explicit `NonDurableFake` representation.
- [x] Every opaque identifier is bounded and canonical; every security digest
  rejects the all-zero sentinel; every revision fits non-wrapping PostgreSQL
  signed `BIGINT` semantics.
- [x] Ports 1.3 replaces `append(AppendCommand)` with typed `transact` and
  `current_head`, uses a store-specific typed error, and remains dependent only
  on Contracts.
- [x] The closed owner set is exactly Project Registry, Task Ledger, Writer
  Lease, Approval Verifier, and Artifact Store. No arbitrary owner, SQL,
  schema/table/column/path/key-value, provider, product, or protected action
  exists.
- [x] The request binds one global transaction ID, one project/snapshot/owner/
  aggregate scope, one domain-command commitment, complete expected daemon
  authority and physical head, record-set/next-state/domain-receipt commitments,
  and optional checkpoint/outbox commitments.
- [x] Store recomputes a canonical domain-separated request digest over every
  field. Same ID/same request returns the identical terminal receipt before
  mutable checks; same ID/changed field rejects without receipt disclosure or
  partial state.
- [x] The fake retains current daemon authority and physical heads independently
  from requests. Only exact fake `ACTIVE` authority and exact physical head
  admit a new normal fake transaction.
- [x] Successful application atomically advances the physical head by exactly
  one checked revision and stores one complete terminal receipt. Stale physical
  head creates a terminal non-mutating denial receipt; authority/admission/
  capacity/corruption failures do not create an authorized mutation.
- [x] Before-apply failure changes nothing. After-apply response loss returns
  typed unknown outcome, and exact retry converges to the retained receipt.
  Serialization conflicts are retried only to one fixed bound.
- [x] Existing exact retries remain readable after new-transaction capacity is
  exhausted. Revision overflow and capacity plus-one deny without mutation.
- [x] Every receipt fixes producer/version, exact request/scope/authority/
  before/after/commitment fields, disposition, transaction digest, and receipt
  digest. Fake receipts are always `RuntimeKind::Fake` plus
  `NonDurableFake`; no live/durable constructor exists in TASK-018.
- [x] A Store physical head/receipt cannot be used as a domain current head,
  approval, lease, effect, task, release, or Guardian authority.
- [x] Project check rejects a current-task marker with no matching unique
  ticket and a current ticket whose named module constitution is missing,
  without requiring inactive future SPEC modules to be activated early.
- [x] The crate performs no filesystem, network, process, environment, clock,
  randomness, database, Git, credential, provider, product, payment,
  publication, deployment, release, or migration I/O.
- [x] `db/migrations/0001_bootstrap.sql` remains byte-for-byte unchanged and
  unexecuted. No PostgreSQL driver, runtime, pool, SQL, or migration runner is
  added.
- [x] AC-32 completion is gated on focused/full verification, independent
  code/security and architecture reviews, and local integration. Durable
  AC-03/04/05/19 and the MVP-1 PostgreSQL exit gate remain open.

## Non-Goals

- Connect to or mutate any PostgreSQL database, install a driver/server, load
  credentials, create roles/schemas/tables, or run a migration.
- Implement durable Task Ledger/outbox, Registry, Lease, Approval, Artifact,
  memory, runtime-admission, or Guardian repositories.
- Decide or duplicate domain transition legality, reconstruct domain receipts,
  execute effects, or provide generic database CRUD/SQL.
- Modify `db/**`, CI, AGENTS rules, other domain source, product files, or a
  companion/playmate website.
- Claim database durability/restart/concurrency/roles/time, MVP-1 completion,
  merge readiness, release, publication, or deployment.
- Commit, push, merge, deploy, or activate a release.

## Module And Constitution Constraints

- Postgres Store 1.0 owns physical transaction mechanics and fake conformance,
  not domain legality or durable truth evidence.
- Contracts 1.8 represents immutable values only and remains serialization/
  hash/I/O free.
- Ports 1.3 remains Contracts-only and exposes no concrete database type.
- The fake may depend only on Contracts, Ports, cjson, and standard in-memory
  collections/errors. It cannot depend on a domain crate or database driver.
- Migration execution requires TASK-019 and a versioned Store amendment.

## Dependencies And Overlap

`parallel_safe: false`: this ticket changes shared Contracts and Ports public
types plus the workspace manifest/lockfile. No concurrent ticket may change
those paths, Store transaction schema, or `ControlStore`.

## TDD Behaviors

1. RED/GREEN: all neutral values enforce exact version, bounded canonical IDs,
   closed owner, complete matching scope, non-zero digests, runtime/durability,
   and signed-BIGINT bounds.
2. RED/GREEN: typed Store trait compiles only with complete request/receipt/
   error/current-head values and retains Contracts-only dependency.
3. RED/GREEN: genesis/current-head lookup and valid transaction atomically
   advance one scope by exactly one revision.
4. RED/GREEN: exact retry returns an equal receipt after later head movement;
   changed transaction-ID reuse and every scope/request field substitution
   reject with zero mutation and no receipt leak.
5. RED/GREEN: stale head returns a stable terminal denial; authority instance,
   epoch, admission, revision, digest, runtime, project, snapshot, owner, and
   aggregate mismatches fail closed.
6. RED/GREEN: exact capacity and plus-one, revision overflow, and retained
   replay after capacity exhaustion behave deterministically.
7. RED/GREEN: before-apply, after-apply unknown, bounded serialization retry,
   and corrupt retained-state faults never guess success.
8. RED/GREEN: canonical request/transaction/receipt digests bind every field;
   fake receipt cannot claim live or durable evidence.
9. REVIEW RED/GREEN: every accepted independent finding receives a failing
   regression before repair.

## Verification

| Check | Command or service | Expected evidence |
|---|---|---|
| Shared contracts | `cargo test -p lattice-contracts --locked` | complete Store values and substitution matrix |
| Port contract | `cargo test -p lattice-ports --locked` | typed Store methods/errors and Contracts-only behavior |
| Store fake | `cargo test -p lattice-postgres-store --locked` | atomicity/retry/authority/head/capacity/fault matrices |
| Format | `cargo fmt --all -- --check` | exit 0 |
| Lint | `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings` | exit 0 |
| Full Rust suite | `cargo test --workspace --all-targets --all-features --locked` | exit 0 |
| Preserved Node suite | `npm.cmd run verify` | exit 0 |
| Dependencies | Cargo metadata/tree plus architecture scan | approved edges only; no driver/domain dependency |
| Zero-I/O scope | source/dependency/credential/SQL/product scans | zero forbidden source/dependency matches |
| Migration inactivity | pre/post SHA-256 and source scan | exact unchanged migration bytes; no runner/load/execute |
| Diff hygiene | `git diff --check` | exit 0 |

## Human Gate

None for this bounded pure/fake local implementation. The user's direct MVP-3
execution instruction authorizes these reversible versioned amendments and
tests. Credentials/accounts/payment, public exposure/publication/deployment,
irreversible deletion or migration, security-control disablement, protected
release, and primary-branch merge remain separately protected.

## Completion Evidence — 2026-08-01

- Focused locked suites pass: Contracts 42, Ports 5, and Postgres Store 14
  (61 total package tests; the Store-specific subsets are 6, 2, and 14).
- Full locked Rust workspace passes 380 tests; preserved Node verification
  passes 44 tests. Format, strict workspace Clippy, governance, dependency,
  forbidden-I/O/SQL/driver/provider/product/website, and diff checks pass.
- The migration remains unchanged and unexecuted at SHA-256
  `7BFF021FC17F738551309C906578C8015B2DD0307D27D239C21DF1697C4D09C8`
  and Git blob `5c1bb61e220980b2087d4ec7a3c61a50a9d23ec5`.
- Independent final code/security and architecture reviews pass with zero
  remaining P0 through P3 findings; local combined integration passes.
- AC-32 is complete only for the typed zero-I/O fake. Durable PostgreSQL,
  migration execution, restart/concurrency, runtime admission, and AC-03/04/
  05/19 remain explicitly open for TASK-019 and later tickets.
