---
ticket_id: TASK-021
spec_id: SPEC-002
spec_version: 23
module_id: postgres-store
constitution_version: 1.3
status: completed
parallel_safe: false
depends_on:
  - TASK-020
allowed_paths:
  - Cargo.toml
  - Cargo.lock
  - crates/lattice-task-ledger/src/lib.rs
  - crates/lattice-task-ledger/tests/task_ledger.rs
  - crates/lattice-postgres-store/Cargo.toml
  - crates/lattice-postgres-store/src/lib.rs
  - crates/lattice-postgres-store/src/live.rs
  - crates/lattice-postgres-store/src/migrations.rs
  - crates/lattice-postgres-store/src/postgres_setup.rs
  - crates/lattice-postgres-store/src/task_ledger.rs
  - crates/lattice-postgres-store/tests/migration_contract.rs
  - crates/lattice-postgres-store/tests/postgres_live.rs
  - crates/lattice-postgres-store/tests/postgres_task_ledger.rs
  - db/migrations/0004_task_ledger_repository.sql
  - scripts/run-task019-postgres.ps1
  - PLANS.md
  - HANDOFF.md
  - docs/PROJECT_CHARTER.md
  - docs/specs/SPEC-002-autonomous-development-platform.md
  - docs/adr/ADR-019-durable-postgres-task-ledger-and-outbox.md
  - docs/modules/V2_AMENDMENT_PROPOSAL.md
  - docs/modules/README.md
  - docs/modules/task-ledger/MODULE_CONSTITUTION.md
  - docs/modules/postgres-store/MODULE_CONSTITUTION.md
  - docs/tickets/TASK-020-postgres-live-control-store.md
  - docs/tickets/TASK-021-postgres-task-ledger-outbox.md
  - docs/workflow/WORKFLOW_LEDGER.md
  - docs/reviews/WORKFLOW_AUDIT_TASK_021_2026-08-02.md
  - docs/reviews/GOVERNANCE_REVIEW_TASK_021_2026-08-02.md
  - docs/reviews/CODE_REVIEW_TASK_021_2026-08-02.md
  - docs/reviews/ARCHITECTURE_REVIEW_TASK_021_2026-08-02.md
  - docs/reviews/INTEGRATION_TASK_021_2026-08-02.md
likely_files:
  - crates/lattice-task-ledger/src/lib.rs
  - crates/lattice-postgres-store/src/task_ledger.rs
  - crates/lattice-postgres-store/src/live.rs
  - crates/lattice-postgres-store/src/migrations.rs
  - crates/lattice-postgres-store/src/postgres_setup.rs
  - crates/lattice-postgres-store/tests/postgres_task_ledger.rs
  - crates/lattice-postgres-store/tests/postgres_live.rs
  - db/migrations/0004_task_ledger_repository.sql
branch: feature/v2-rust-postgres-bootstrap
---

## Objective

Implement the first durable domain repository: one Task-Ledger-planned live
PostgreSQL append that atomically commits the complete command and terminal
receipt, optional event and effect-intent outbox admission, resulting
head/resource projection/checkpoint, and applied physical Store receipt. Prove
restart replay and fail-closed corruption while preserving every TASK-020
physical receipt across global schema v3.

## Acceptance Criteria

- [x] Task Ledger 2.1 provides one runtime-aware vacant/plan/apply boundary;
  the fake uses it and every existing request/event/head/receipt hash and
  behavior remains unchanged.
- [x] `VerifiedStream` retains verified appended and denied commands, exports
  complete untrusted command/event/outbox state, and supports exact typed
  receipt lookup after restart.
- [x] One complete deterministic checkpoint binds identity, head, resource
  projection, ordered events, every terminal command including denials, and
  all outbox admissions. Exact retry leaves it unchanged; every new terminal
  command changes it exactly once.
- [x] Only a successfully appended `EFFECT_INTENT` with audit outcome
  `RECORDED` derives one immutable outbox admission whose intent digest equals
  the event subject digest. Existing non-`RECORDED` outcomes remain valid
  appended events but derive no admission; denied and non-effect commands also
  derive none.
- [x] Applying a plan rechecks the complete base checkpoint; stale plans,
  wrong checkpoints, denial-tail rollback, reordered/truncated/injected
  command/event/outbox records, and projection disagreement fail closed.
- [x] `0001` through `0003` remain byte-identical. Exact transaction-control-
  free `0004_task_ledger_repository.sql` advances global schema/compatibility
  to v3 and creates only the four approved Ledger/outbox tables and eight new
  fixed functions.
- [x] Global v3 evidence and the immutable Store-v2 receipt profile are
  distinct. `store_*_v3` verifies v3 but reconstructs old and new Store v2
  receipts using physical schema profile 2 plus first-three-entry manifest
  `4582edce68a947998a8f4c6895bb37ceec9e842f516471f4d9e2617a6757f129`.
- [x] The historical `store_*_v2` functions remain exact catalog history but
  have zero runtime EXECUTE. Runtime has exact non-grantable EXECUTE only on
  the three Store-v3 and five Task-Ledger-v1 functions and zero direct
  SELECT/DML on all protected tables.
- [x] The runner accepts Fresh, exact v1 prefix, exact v2 prefix, or exact v3
  full state. v1 preserves its empty precondition; non-empty v2 upgrades only
  after exact source verification and STOPPED/no-leader admission. ACTIVE or
  any drift/partial/unknown/reordered/edited source fails closed.
- [x] Ledger authoritative fields use fixed columns; only sanitized bounded
  diagnostic uses `jsonb`, rejects JSON numbers, and is reconstructed and
  revalidated in Rust before hashing. PostgreSQL JSON text is never hash input.
- [x] Every Ledger `u64` persists as constrained `numeric(20,0)` and crosses
  SQL as canonical decimal text. Store physical revision remains checked
  signed `BIGINT`.
- [x] `PostgresTaskLedger` consumes only a caller-supplied authenticated
  runtime client and exact target, exposes no raw client/query/SQL/DSN/
  credential/environment surface, and delegates all domain construction and
  replay to Task Ledger 2.1.
- [x] A new command runs in one bounded `SERIALIZABLE` transaction. Exact retry
  or changed reuse is classified before mutable admission; new work rechecks
  exact ACTIVE daemon authority and locked physical state.
- [x] New work calls fixed `store_finalize_v3` then fixed
  `task_ledger_finalize_v1` inside that same transaction. The Ledger finalizer
  rechecks the base checkpoint and exact Store terminal/request; any failure
  rolls back both. No composite-row argument or extra runtime type/table grant
  is introduced.
- [x] Command, optional event/outbox, projection/checkpoint, and applied Store
  receipt are committed together or not at all. A domain stale/overflow denial
  creates no event/outbox but still persists one terminal command and advances
  the complete physical checkpoint exactly once.
- [x] The physical Store transaction ID is exactly
  `task-ledger-v1:<sha256>` over the frozen canonical owner/stream/command
  subject. It is stable after restart/unknown commit outcome and distinct for
  different streams and repository-owner namespaces without truncation.
- [x] Store scope/mutation mapping exactly follows ADR-019: owner/aggregate are
  `TASK_LEDGER`/complete stream ID; request, record-set, checkpoint, and receipt
  commitments map field for field; both next-state and optional checkpoint use
  the next checkpoint; optional outbox intent uses the Outbox Admission digest
  while the row separately retains the event-subject intent digest.
- [x] A schema-v3 Ledger exact retry after later events, STOPPED admission,
  daemon epoch change, restart, or commit-response loss reconstructs identical
  domain and Store receipts. Separately, a historical Store-v2 receipt replays
  byte-identically after v2-to-v3 upgrade. A changed command returns no retained
  receipt.
- [x] Serialization/deadlock/fixed first-row races receive at most three
  pre-commit retries. Commit response failure returns no receipt, poisons the
  instance, and requires a new client plus exact retry.
- [x] Fresh/v1/v2/v3 migration, old Store replay, append/denial/outbox,
  same-command/same-stream/cross-stream concurrency, fault rollback,
  exhaustion, unknown commit, restart, corruption, ACL, and service-safe
  cleanup pass in the marker-owned PostgreSQL 17.10 harness.
- [x] Full Rust/Node verification, format, strict Clippy, dependency tree,
  `cargo audit`, scope/secret/dynamic-SQL scans, independent code/security and
  architecture reviews, local integration, ledger, and handoff pass before
  completion.
- [x] SPEC-002 AC-03, AC-04, and AC-35 close only after the direct durable
  evidence above. AC-05, AC-19, MVP-1, MVP-2, and MVP-3 remain open.

## Non-Goals

- Issue or persist live Task Ledger resource observations; claim/deliver/retry
  an outbox effect; call a provider; or claim exactly-once effects.
- Implement Registry, Writer Lease, Approval, Artifact/filesystem, Task Domain
  projection, Orchestrator, Review Runtime, Workspace Git, or Scope Check.
- Install or invoke OpenClaw, Codex, Graphify, Hermes, Codebase Memory, or any
  unrelated companion/playmate website component.
- Activate/elect a daemon or Guardian, create production credentials/database,
  connect remotely/TLS, replace a service, publish, release, or deploy.
- Add generic SQL/JSON row mutation, a raw client getter, dynamic migration
  discovery, automatic startup migration, SQLx, Diesel, pools, or direct async
  ports.
- Commit, push, merge, reset, clean, switch branch, or mutate a production or
  user database.

## Module And Constitution Constraints

- Task Ledger 2.1 alone owns append, event, receipt, outbox admission,
  projection, replay, and checkpoint semantics and remains zero-I/O.
- Postgres Store 1.3 owns only physical persistence/transaction/catalog/error
  mechanics and invokes the Task Ledger public planner/verifier.
- Dependency direction is one-way `postgres-store -> task-ledger`; Ports and
  Contracts remain unchanged and no concrete adapter calls another adapter.
- The Store-v2 receipt profile is immutable historical evidence and must not be
  rebound to the global v3/full manifest.
- One Gateway / One Truth / One Writer and project/snapshot/stream isolation
  remain mandatory.

## Dependencies And Overlap

`parallel_safe: false`: Task Ledger public semantics, Postgres Store, the exact
migration manifest/verifier, and the disposable live harness change as one
compatibility unit. Coordinated implementation may use disjoint files, but no
other ticket may concurrently modify these modules or schema.

## TDD Behaviors

1. RED/GREEN pure runtime-aware vacant/plan/apply, retained command lookup,
   checkpoint, outbox derivation, corruption, and fake-parity matrices.
2. RED/GREEN four-entry manifest, v1/v2 prefix classification, frozen Store
   profile, v3 exact no-op, STOPPED upgrade, rollback, and concurrent runner.
3. RED/GREEN exact four-table/eight-new-function catalog, source/config/ACL,
   historical-function revocation, numeric bounds, and direct-table denial.
4. RED/GREEN live new/exact/changed/stale/overflow/effect/non-effect command,
   projection/checkpoint, same/cross-stream concurrency, and restart replay.
5. RED/GREEN every transaction fault point, serialization exhaustion,
   commit-response loss/poison/reconnect, partial-pair and retained corruption.
6. REVIEW RED/GREEN every accepted independent finding before repair.

## Verification

| Check | Expected evidence |
|---|---|
| Task Ledger focused tests | pure plan/checkpoint/outbox/replay/fake parity; existing hashes unchanged |
| Postgres Store focused tests | manifest profiles, adapter/error/retry/static conversion behavior |
| Disposable PostgreSQL harness | actual 17.10 migration/ACL/atomicity/concurrency/fault/restart/corruption evidence |
| Historical Store replay | v2 receipt before upgrade equals v3-adapter exact replay byte for byte |
| Full Rust/Node | every prior behavior plus TASK-021 passes |
| Format and strict Clippy | exit 0, zero warnings |
| Cargo tree/audit | only approved Store-to-Ledger and exact serde-json support edges; zero known advisory |
| Migration/catalog scans | `0001`-`0003` unchanged; exact `0004`; fixed tables/functions/ACLs only |
| Secret/connection scans | zero credential/DSN/environment/raw-driver retention; marker target only |
| Git/diff/governance | no conflict/whitespace errors; exact current marker and ticket; dirty baseline preserved |

## Human Gate

None for reversible code/tests and the marker-owned disposable local cluster;
the user already approved the durable V2 module direction and direct execution
through MVP-3. Production provisioning/credentials, non-loopback exposure,
destructive or incompatible migration, real daemon/Guardian activation,
security-control changes, protected release, and primary-branch merge remain
separate protected actions and are not performed by TASK-021.
