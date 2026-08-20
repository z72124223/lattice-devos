---
module_id: postgres-approval-verifier
name: PostgreSQL Approval Verifier Adapter
version: 1.0
status: active
owner: LATTICE maintainers
last_reviewed: 2026-08-20
---

## Mission

Persist the Approval Verifier 1.1 global aggregate through one exact,
checksummed PostgreSQL extension; supply database time and current runtime
admission inside serializable transactions; replay every untrusted snapshot
against independently retained checkpoints; and atomically bind one normal
approval consume receipt to one exact immutable effect claim.

## Non-Goals

- Decide subject, proof, nonce, time-window, revocation, lane, current-head,
  retry, or claim legality.
- Authenticate OS users, Guardian trust roots, raw keys, tokens, nonces, proof
  assertions, or credentials.
- Expose or consume a protected approval, run Guardian activation, or set
  `DRAINING`.
- Execute the claimed effect, mutate Task Ledger/Registry/Policy/Artifact data,
  or grant a generic cross-module transaction API.
- Accept caller SQL, DSN, credential, schema/table/function name, observation
  time, daemon identity/epoch, admission, or authority Boolean.
- Operate on a production/user database, public listener, deployment, release,
  payment, account, credential, or protected branch.

## Owned Data

- Extension SQL/manifest identity and append-only extension ledger.
- One global aggregate snapshot, byte hash, row version, and independent
  Approval checkpoint columns.
- Immutable physical command rows with exact repository-intent, pure command,
  terminal receipt, digests, and outcomes.
- Immutable normal effect-claim rows binding effect intent, database
  observation, current daemon/admission, domain receipt, and claim digest.

Approval Verifier owns every semantic value. Postgres Store owns the global
database/admission profile. This adapter only observes Store-owned rows and
never mutates or reinterprets them.

## Public Contracts

- `PostgresApprovalVerifier` implements only Approval Verifier's
  `ApprovalRepository` trait through typed Rust inputs.
- Setup accepts one prevalidated target identity and supports fresh install or
  exact-current verification only.
- Runtime calls a fixed v1 function allowlist; direct table reads/writes are
  not part of the public contract.
- `execute`, `claim_normal`, and `current_authority` create their own bounded
  serializable transactions and return stable closed repository failures.

## Invariants

1. One locked global row serializes every command and therefore every nonce
   binding across approval/project/lane boundaries.
2. Stored snapshot bytes are untrusted until bounded hash verification, full
   domain replay, independent checkpoint comparison, and physical row closure.
3. PostgreSQL `transaction_timestamp()` is the only durable observation time.
   The current exact ACTIVE daemon/epoch/admission is loaded in the same
   transaction and cannot be supplied by callers.
4. Domain planning/apply is the only state transition path. SQL validates
   expected versions/digests and atomic row shapes but never decides semantic
   legality or synthesizes receipts.
5. Normal domain consume, command receipt, aggregate head, and exact effect
   claim commit together or not at all. Protected denial writes no effect row.
6. Exact retry compares original canonical repository-intent bytes before new
   observations. Changed intent, command-ID reuse, corrupt storage, or
   ambiguous authority fails closed.
7. Commit-response uncertainty never returns success. Recovery requires a
   fresh client and exact retry; no in-memory result grants authority.
8. Runtime has schema usage and exact function execution only. Tables,
   sequences, generic DML, PUBLIC, readonly, and unrelated roles have zero
   approval data privileges.
9. Raw nonce/token/key/assertion/credential bytes never enter SQL, state,
   logs, errors, receipts, or debug output.
10. Setup/runtime accept no production path or arbitrary database target from
    an Approval repository command.

## Allowed Dependencies

- Rust standard library.
- `lattice-approval-verifier` 1.1.
- `lattice-contracts` immutable shared identities.
- Exact pinned `postgres`, `sha2`, and `time` crates.
- Embedded `db/extensions/approval-verifier/v1.sql` bytes.

## Forbidden Dependencies

- Policy, Task Domain, Task Ledger, Project Registry, Writer Lease, Guardian,
  Artifact Store, Orchestrator, ports, provider adapters, filesystem/Git, CLI,
  product repositories, or another concrete persistence adapter.

## Failure, Compatibility, And Migration

Fresh install and exact-current v1 no-op are accepted. Empty-partial,
half-installed, extra, wrong-owner/ACL/catalog/function/identity/ledger,
wrong-database/global-profile, corrupt aggregate/command/effect, rollback,
substitution, serialization exhaustion, and commit ambiguity fail closed.
There is no automatic repair, drop, rewrite, downgrade, or recursive cleanup.

Future versions are append-only successors. They must retain every accepted v1
SQL byte, manifest, command/receipt/snapshot/checkpoint/effect-claim byte and
high-water unless a separately approved migration ADR proves compatibility.

## Acceptance Gates

| Gate | Evidence | Owner | Required for merge |
|---|---|---|---|
| Embedded identity | exact SQL length/hash and manifest hash | Engineering | yes |
| Catalog/ACL closure | exact relation/column/constraint/index/function/owner/ACL profiles | Security review | yes |
| Domain delegation | source/API tests prove public planner/replay only; no semantic SQL | Architecture review | yes |
| Global nonce/replay | concurrent cross-approval nonce plus corruption/rollback matrices | Security review | yes |
| Normal claim atomicity | two claimers, protected denial, changed intent, exact retry, effect-row closure | Engineering | yes |
| Failure/restart | rollback, serialization, commit-unknown, fresh connection/process and PostgreSQL restart | Engineering | yes |
| Secrets/surface | leak scan, fixed functions, no direct DML/SQL/DSN/credential inputs | Security review | yes |
| Full verification | focused/live tests, workspace tests, strict Clippy, format, project and diff checks | Engineering | yes |

## Change Policy

Schema/function/ACL, aggregate/checkpoint/command/effect persistence, time or
admission source, retry/ambiguity, repository API, dependency, protected-lane,
or secret boundary changes require a versioned constitution and SPEC/ADR trace.

## Amendment History

| Version | Date | Decision reference | Summary | Approver |
|---|---|---|---|---|
| 1.0 | 2026-08-20 | SPEC-002 v36, ADR-024, TASK-024 | Independent global Approval aggregate repository and atomic normal effect claim; no protected claim or live authentication | User TASK-023-025 development directive |
