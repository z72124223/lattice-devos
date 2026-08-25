---
ticket_id: TASK-022
spec_id: SPEC-002
spec_version: 24
module_id: postgres-store
constitution_version: 1.4
status: completed
parallel_safe: false
depends_on:
  - TASK-021
allowed_paths:
  - README.md
  - Cargo.toml
  - Cargo.lock
  - crates/lattice-project-registry/Cargo.toml
  - crates/lattice-project-registry/src/lib.rs
  - crates/lattice-project-registry/tests/project_registry.rs
  - crates/lattice-postgres-store/Cargo.toml
  - crates/lattice-postgres-store/src/lib.rs
  - crates/lattice-postgres-store/src/live.rs
  - crates/lattice-postgres-store/src/migrations.rs
  - crates/lattice-postgres-store/src/postgres_setup.rs
  - crates/lattice-postgres-store/src/task_ledger.rs
  - crates/lattice-postgres-store/src/project_registry.rs
  - crates/lattice-postgres-store/tests/migration_contract.rs
  - crates/lattice-postgres-store/tests/postgres_live.rs
  - crates/lattice-postgres-store/tests/postgres_store.rs
  - crates/lattice-postgres-store/tests/postgres_task_ledger.rs
  - crates/lattice-postgres-store/tests/postgres_project_registry.rs
  - db/migrations/0005_project_registry_repository.sql
  - scripts/run-task019-postgres.ps1
  - PLANS.md
  - HANDOFF.md
  - docs/PROJECT_CHARTER.md
  - docs/specs/SPEC-002-autonomous-development-platform.md
  - docs/adr/ADR-016-typed-zero-io-postgres-store-boundary.md
  - docs/adr/ADR-020-durable-postgres-project-registry.md
  - docs/modules/V2_AMENDMENT_PROPOSAL.md
  - docs/modules/README.md
  - docs/modules/project-registry/MODULE_CONSTITUTION.md
  - docs/modules/postgres-store/MODULE_CONSTITUTION.md
  - docs/tickets/TASK-022-postgres-project-registry.md
  - docs/workflow/WORKFLOW_LEDGER.md
  - docs/reviews/WORKFLOW_AUDIT_TASK_022_2026-08-03.md
  - docs/reviews/GOVERNANCE_REVIEW_TASK_022_2026-08-03.md
  - docs/reviews/GOVERNANCE_REREVIEW_TASK_022_2026-08-03.md
  - docs/reviews/CODE_REVIEW_TASK_022_2026-08-03.md
  - docs/reviews/ARCHITECTURE_REVIEW_TASK_022_2026-08-03.md
  - docs/reviews/INTEGRATION_TASK_022_2026-08-03.md
likely_files:
  - crates/lattice-project-registry/src/lib.rs
  - crates/lattice-project-registry/tests/project_registry.rs
  - crates/lattice-postgres-store/src/project_registry.rs
  - crates/lattice-postgres-store/src/live.rs
  - crates/lattice-postgres-store/src/task_ledger.rs
  - crates/lattice-postgres-store/src/migrations.rs
  - crates/lattice-postgres-store/src/postgres_setup.rs
  - crates/lattice-postgres-store/tests/postgres_project_registry.rs
  - crates/lattice-postgres-store/tests/postgres_live.rs
  - db/migrations/0005_project_registry_repository.sql
branch: feature/v2-rust-postgres-bootstrap
---

## Objective

Implement one durable Project-Registry-planned PostgreSQL repository. Preserve
Project Registry as the sole owner of global project identity, lifecycle,
accepted/pending reservations, collision blocking, reconciliation, immutable
authority receipts, command replay, and checkpoint meaning. Persist the exact
pure plan in one bounded Registry-specific global transaction without
pretending the global aggregate is a project-scoped `StoreScope`.

## Acceptance Criteria

- [ ] Project Registry 1.2 exposes one runtime-aware verified global state,
  `plan -> apply` boundary, and Fake wrapper while preserving every existing
  Fake authority/command receipt digest and TASK-012 behavior.
- [ ] Before planner extraction, literal golden vectors freeze representative
  Registry 1.1 observation, request, authority-receipt, and command-result
  digests. The existing `result_digest` is the terminal semantic commitment;
  checkpoint and record-set vectors are new Registry 1.2 subjects.
- [ ] One immutable Registry checkpoint binds runtime, non-wrapping global
  command ordinal, every complete command request/terminal receipt, all current
  project projections, accepted/pending reservations, and deterministic counts.
- [ ] Vacant high-water is zero, first-seen records are exactly `1..N`, and the
  103-byte vacant logical-state plus Fake/Live checkpoint digests match the
  literals frozen in ADR-020.
- [ ] Every first-seen terminal command, including `Denied`, `Blocked`, and an
  exact no-project-change observation, advances the global ordinal/checkpoint
  exactly once. Exact command/request replay changes neither; changed command
  reuse returns no retained receipt.
- [ ] Exported untrusted snapshots are verified by ordered replay from a vacant
  state. Plain verification proves self-consistency only; durable currentness
  uses `RegistryCheckpoint::from_retained` and
  `verify_untrusted_registry_snapshot_against_checkpoint`. Missing/reordered/
  duplicated/injected commands, denial-tail rollback, project/authority/
  pending/drift disagreement, reservation collision, count, checkpoint, or
  runtime substitution fails closed.
- [ ] The pure planner remains the only constructor of registration,
  observation, suspension, collision blocking, reconciliation, snapshot,
  reservation, receipt, record-set, and checkpoint meaning. It performs no I/O.
- [ ] Checkpoint command core, logical retained state, record-set persistence
  core, transaction digest, and persistence receipt follow ADR-020's acyclic
  construction order. Checkpoints/record sets exclude all adapter evidence and
  no digest is an input to itself.
- [ ] The Registry-owned limits are versioned and fail closed: at most 4,096
  current projects, 65,536 first-seen terminal commands, 67,108,864 retained
  snapshot bytes, and 131,072 UTF-8 bytes in one already-NFC canonical root.
  Retained bytes are the exact canonical logical-state algorithm from ADR-020:
  unique observations once, canonical collections, explicit nulls, and no
  checkpoint/record-set/adapter fields. Exact replay/changed-ID classification
  precedes capacity checks; TASK-022 adds no deletion, compaction, or silent
  truncation policy.
- [ ] `0001` through `0004` remain byte-identical. Exact transaction-control-
  free `0005_project_registry_repository.sql` advances only an exact accepted
  source to global schema v4 and adds the five approved Registry tables.
- [ ] Schema v4 preserves Store-v2 receipt profile 2 and its first-three-entry
  manifest commitment. Historical physical Store and Task Ledger domain
  receipts replay byte-identically after v3-to-v4 upgrade.
- [ ] Schema v4 replaces the runtime write surface with forward-profile-bound
  Store-v4 and Task-Ledger-v2 fixed functions, retains older functions as
  ungranted catalog history, and adds only the fixed Registry-v1 functions
  frozen by ADR-020.
- [ ] Exact catalog totals are 15 `control` tables, 28 retained functions, 17
  runtime-executable successor functions, and all 11 historical functions
  retained without runtime EXECUTE. Registry's nine scalar signatures are
  exactly the ADR-020 manifest and have a maximum of 73 inputs.
- [ ] Runtime has zero direct SELECT/DML on every protected table. Every
  executable function is exact-signature, migrator-owned, `SECURITY DEFINER`,
  schema-qualified, dynamic-SQL-free, non-leakproof, parallel-unsafe,
  row-security-on, timeout-bounded, and non-grantable only to runtime.
- [ ] The Registry schema uses fixed authoritative columns. No authoritative
  Registry state, command, observation, authority, denial, drift, checkpoint,
  or reservation is stored as `jsonb`, arbitrary maps, opaque canonical blobs,
  or caller-defined SQL.
- [ ] `0005` seeds one exact Live vacant singleton with high-water/counts zero,
  retained bytes 103, and the frozen checkpoint digest; the other four tables
  start empty. That singleton is the first-command serialization/checkpoint
  point. Immutable complete observation, project, command, and normalized
  accepted/pending identity-reservation rows are cross-checked against pure
  replay before any authority is returned; missing, extra, or orphaned
  observations fail closed.
- [ ] A new command runs in one bounded `SERIALIZABLE` transaction with
  5-second lock, 30-second statement, 30-second idle-in-transaction, and
  45-second monotonic pre-commit limits. Exact
  replay/changed-ID classification precedes mutable admission; new work checks
  the exact ACTIVE daemon authority, admission, global profile, and locked base
  checkpoint in the same transaction.
- [ ] Fixed command/project staging plus finalization occurs only inside that
  Rust transaction. Finalization accepts only rows written by the current
  transaction, checks the exact plan/base/result/record-set shape, and commits
  command, optional project/reservations, state checkpoint, and persistence
  evidence together or not at all.
- [ ] A directly committed partial/staged row can never become Registry
  authority. It makes the retained state corrupt and fail closed; the adapter
  never silently repairs, completes, or deletes it.
- [ ] `PostgresProjectRegistry` consumes only a caller-supplied authenticated
  runtime client and exact verified target. It exposes no SQL, table, raw
  client, DSN, password, environment discovery, generic row, or migration API.
- [ ] Durable execution returns a semantic Registry receipt only after commit
  plus distinct database identity/global schema/checkpoint evidence. It does
  not fabricate a Store receipt or a project authority snapshot for a global
  registration denial.
- [ ] Commit failure with no database response returns no receipt, poisons the
  adapter, and converges only through a new client plus exact request. Explicit
  database responses remain known retryable or terminal outcomes; bounded
  serialization/deadlock retries occur only before outcome uncertainty.
- [ ] Concurrent same-command, same-project, cross-project duplicate identity,
  pending-reservation front-run, collision-blocking, and unrelated registration
  matrices serialize to one legal pure-domain history with byte-identical
  replay and no second reservation.
- [ ] Fresh and exact v1/v2/v3/v4 migration/no-op, non-empty stopped-v3 upgrade,
  ACTIVE denial, rollback, concurrent runner, restart, commit-ack loss,
  timeout, ACL, manifest drift, corruption, partial-stage, and service-safe
  cleanup pass in the marker-owned PostgreSQL 17.10 harness.
- [ ] Full Rust/Node verification, format, strict Clippy, dependency tree,
  `cargo audit`, scope/secret/dynamic-SQL scans, independent code/security and
  architecture reviews, local integration, ledger, and handoff pass before
  completion.
- [ ] SPEC-002 AC-36 closes only after the direct durable evidence above.
  AC-06 remains open for real Windows/Git inspection, Workspace Git, changed-
  path evidence, and Scope Check. MVP-1, MVP-2, and MVP-3 remain open.

## Non-Goals

- Inspect a real Windows path, file ID, junction, Git repository, loose/packed
  ref, commit, worktree, changed path, hook/driver, or conflict.
- Implement Writer Lease, Approval, Artifact/filesystem, Workspace Git, Scope
  Check, Orchestrator, Review Runtime, provider/product, or effect delivery.
- Add a global variant to Contracts/Ports/Store receipts, forge a sentinel
  `ProjectId`/`ProjectSnapshotId`, or reinterpret per-project `StoreScope`.
- Install or invoke OpenClaw, Codex, Graphify, Hermes, Codebase Memory, or any
  unrelated companion/playmate website component.
- Activate/elect a daemon or Guardian, create production credentials/database,
  connect remotely/TLS, replace a service, publish, release, or deploy.
- Commit, push, merge, reset, clean, switch branch, or mutate a production or
  user database.

## Module And Constitution Constraints

- Project Registry 1.2 owns all global Registry semantic planning, replay,
  projection, reservation, receipt, and checkpoint behavior and stays zero-I/O.
- Postgres Store 1.4 owns only migration/catalog/ACL/client/transaction/locking/
  retry/poison/static-conversion/durability mechanics plus the explicit global
  Registry persistence exception.
- Dependency direction is one-way
  `lattice-postgres-store -> lattice-project-registry`; no domain owner imports
  Postgres Store and no concrete adapter calls another adapter.
- ADR-020 narrowly amends ADR-016: global Registry commands use typed Registry
  persistence evidence, not a false project-scoped physical Store receipt.
- One Gateway / One Truth / One Writer and project isolation remain mandatory.

## Dependencies And Overlap

`parallel_safe: false`: this ticket changes the global migration profile,
runtime function allowlist, schema verifier, Project Registry public planner,
Postgres Store dependency set, and the shared PostgreSQL harness. No other
ticket may change these files or interfaces concurrently.

## TDD Behaviors

1. CHARACTERIZATION/GREEN then RED/GREEN: freeze the actual Registry 1.1
   observation/request/authority/result vectors; then add the new 1.2 vacant
   checkpoint, record-set, export/verify, independent retained-checkpoint, byte-
   accounting, boundary, and corruption matrices.
2. RED/GREEN: command planning is embedded in `FakeProjectRegistry`; extract one
   pure plan/apply path and prove all old Fake receipts/hashes unchanged.
3. RED/GREEN: new terminal commands advance one global ordinal/checkpoint while
   exact replay is stable and denial-tail/reorder/substitution fails closed.
4. RED/GREEN: schema-v4 manifest/catalog/ACL contract is absent; add immutable
   `0005`, the seeded vacant singleton plus four empty Registry tables,
   profile-bound Store/Ledger successors, the exact `15/28/17/11-ungranted`
   catalog totals, and nine Registry signatures with 73 maximum inputs.
5. RED/GREEN: `PostgresProjectRegistry` load/execute is absent; add pure replay,
   exact replay/changed-ID ordering, ACTIVE authority, and atomic finalization.
6. RED/GREEN: registration, exact observation, drift, suspension,
   reconciliation, collision blocking, reservation front-run, and command
   replay match Fake under live PostgreSQL.
7. RED/GREEN: concurrency, rollback, timeout, commit-response loss/reconnect,
   partial-stage, row/checkpoint/reservation corruption, and restart fail closed.
8. RED/GREEN: v3 Store/Ledger history survives v4 and all runtime/table/ACL/
   manifest boundaries remain exact.
9. REVIEW RED/GREEN: every actionable independent review finding receives a
   failing regression before repair.

## Verification

| Check | Command or service | Expected evidence |
|---|---|---|
| Pure Registry | `cargo test -p lattice-project-registry --locked` | planner/checkpoint/replay/Fake parity passes |
| Registry adapter | `cargo test -p lattice-postgres-store --test postgres_project_registry --locked` | typed adapter matrices pass |
| Store package | `cargo test -p lattice-postgres-store --locked` | migration/Store/Ledger/Registry compatibility passes |
| Live PostgreSQL | `powershell -File scripts/run-task019-postgres.ps1` | owned PostgreSQL 17.10 initial/restart matrix passes |
| Format | `cargo fmt --all -- --check` | exit 0 |
| Lint | `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings` | exit 0 |
| Full Rust | `cargo test --workspace --all-targets --all-features --locked` | exit 0 |
| Preserved Node | `npm.cmd run verify` | exit 0 |
| Dependencies | `cargo tree -p lattice-postgres-store --edges normal --locked` | only approved one-way domain edges |
| Security/hygiene | exact scans, `cargo audit`, `git diff --check` | zero forbidden findings; exit 0 |

## Human Gate

None for this bounded, reversible local implementation and marker-owned
disposable PostgreSQL verification. The approved V2 amendment plans one domain
repository at a time and the user directed continued execution through MVP-3.
Credentials, account/payment actions, public exposure, irreversible actions,
security-control changes, real project mutation, protected release activation,
primary-branch merge, publication, and deployment remain outside this ticket.

## 2026-08-25 reconciliation

TASK-022 is completed by the later TASK-075 integration rather than by simply
assuming that this ticket's unchecked historical checklist passed. TASK-075
binds the original implementation commit `12f7100`, closure commit `a1aced9`,
and integrated product commit `a3599c1`; it also records the migration,
fresh/restart replay, ACL, and wrong-ordinal rejection evidence. The current
product still contains the exact Registry migration/library payload and the
integrated PostgreSQL adapter. PR #12 was merged into that accepted lineage.

The remaining unchecked broad cross-module items above are historical planning
detail and are not relabelled as independently rerun on 2026-08-25.
