---
ticket_id: TASK-014
spec_id: SPEC-002
spec_version: 10
module_id: writer-lease
constitution_version: 1.0
status: completed
parallel_safe: false
depends_on:
  - TASK-013
allowed_paths:
  - Cargo.toml
  - Cargo.lock
  - crates/lattice-contracts/**
  - crates/lattice-policy/**
  - crates/lattice-writer-lease/**
  - PLANS.md
  - HANDOFF.md
  - docs/specs/SPEC-002-autonomous-development-platform.md
  - docs/adr/ADR-009-policy-decision-facts-and-fail-closed-intents.md
  - docs/adr/ADR-012-writer-lease-authority-and-recovery.md
  - docs/modules/V2_AMENDMENT_PROPOSAL.md
  - docs/modules/README.md
  - docs/modules/lattice-contracts/**
  - docs/modules/policy-engine/**
  - docs/modules/writer-lease/**
  - docs/tickets/TASK-014-writer-lease.md
  - docs/workflow/WORKFLOW_LEDGER.md
  - docs/reviews/WORKFLOW_AUDIT_TASK_014_2026-07-29.md
  - docs/reviews/CODE_REVIEW_TASK_014_2026-07-29.md
  - docs/reviews/ARCHITECTURE_REVIEW_TASK_014_2026-07-29.md
  - docs/reviews/INTEGRATION_TASK_014_2026-07-29.md
likely_files:
  - Cargo.toml
  - Cargo.lock
  - crates/lattice-contracts/src/lib.rs
  - crates/lattice-contracts/tests/contracts.rs
  - crates/lattice-policy/src/types.rs
  - crates/lattice-policy/src/checks.rs
  - crates/lattice-policy/tests/policy_matrix.rs
  - crates/lattice-writer-lease/Cargo.toml
  - crates/lattice-writer-lease/src/lib.rs
  - crates/lattice-writer-lease/tests/writer_lease.rs
branch: feature/v2-rust-postgres-bootstrap
---

## Objective

Implement Writer Lease 1.0 as a pure Rust semantic owner with reusable
transition planning, complete untrusted aggregate verification, exact command
idempotency, signed-BIGINT-safe fencing, typed expiry/recovery, fixed-producer
authority receipts, a deterministic non-durable fake, and Policy 2.5 owner
receipt/current-head consumption.

## Acceptance Criteria

- [x] Contracts 1.4 defines fixed `lattice-writer-lease`/`1.0` producer values,
  closed runtime-admission representation, positive `i64` epoch/fence/revision
  types, complete lease identity, and full authority receipt/head projection.
- [x] Identity exactly binds project/snapshot/task/revision/spec/attempt,
  lease/holder/worktree/process/start identity, daemon instance/epoch, and
  fence; malformed, non-canonical, empty, oversized, or substituted fields
  reject.
- [x] The public pure planner and verified-aggregate API own all acquire,
  heartbeat, suspect, release, revoke, fence allocation, hashing, and command
  retry meaning; future stores cannot need a second transition implementation.
- [x] One project permits at most one active/suspect writer. Conflicting
  acquire returns stable terminal denial with no partial mutation; different
  projects remain isolated.
- [x] Fence/epoch/revision values are `1..=i64::MAX`; overflow, rollback, or
  reuse rejects before mutation. Release/revoke/reacquire allocates a strictly
  newer fence.
- [x] Same project/command/request returns the identical terminal receipt
  before stale-head evaluation, including after later transitions. Changed
  content under one command ID rejects. Applied and denied receipts form one
  predecessor-bound command chain with a separate high-water/tail commitment.
- [x] Expiry can only mark suspect; heartbeat cannot revive suspect; exact
  release and evidence-bound holder-death/newer-epoch revoke are distinct and
  preserve immutable transition evidence.
- [x] The complete runtime-admission matrix denies heartbeat during draining,
  all user-project transitions during canary/stopped, and ordinary work during
  reconciliation-required.
- [x] Public untrusted aggregate replay rejects unknown version, malformed
  record, reorder, truncation, duplication, orphan receipt, hash/head/counter/
  authority substitution, and claimed-state disagreement. Rollback-sensitive
  restore additionally requires an independently retained validated checkpoint;
  it may not derive that checkpoint from the raw snapshot being checked.
- [x] Policy 2.5 removes caller-owned writer active/current/role/current-epoch/
  current-fence/active-count fields, consumes the shared owner receipt plus an
  independent optional current head, and denies every binding/identity/
  status/runtime/admission/head/digest substitution.
- [x] Writer Lease has no Task Domain, Policy, Registry, Ledger, ports, store,
  filesystem, database, process, network, environment, random, credential,
  provider, payment, publication, deployment, or product-repository
  dependency/I/O.
- [x] AC-05 stays open: the fake does not claim PostgreSQL concurrent
  acquisition, DB clock, restart, stale live connection, or atomic durable
  mutation fencing.

## Non-Goals

- Connect to PostgreSQL, define migrations/tables/indexes/roles, or prove
  transactions, concurrency, restart, database time, unknown commit, or
  stale-connection rejection.
- Read a process table, kill a holder, advance daemon leadership/admission, or
  authenticate recovery evidence.
- Create worktrees, write product files, run a provider, or change
  credentials/accounts/payment/publication/deployment state.
- Import V1 ProjectLock files/counters as authority.
- Commit, push, merge, or activate a protected release.

## Module And Constitution Constraints

- Writer Lease 1.0 owns pure lease/fence/recovery semantics and depends only
  on Contracts 1.4, cjson mechanics, and exact time parsing/formatting.
- Contracts 1.4 owns neutral immutable values only; it does not issue, hash,
  persist, authenticate, or decide lease transitions.
- Policy 2.5 owns deterministic sufficiency only and has no normal Writer Lease
  dependency. A test-only dependency may obtain the fake owner's actual
  current head.
- Runtime admission and daemon epoch transitions remain Guardian/PostgreSQL
  owned; Writer Lease consumes only explicit observations/evidence.

## Dependencies And Overlap

`parallel_safe: false`: this ticket changes shared Contracts and Policy public
types, the workspace lockfile, and a new owner crate. No other ticket may
modify these paths or interfaces concurrently.

## TDD Behaviors

1. RED/GREEN: shared receipt/head and positive signed-BIGINT types are absent,
   then reject producer/version/value/full-field substitution.
2. RED/GREEN: Writer Lease crate/planner/fake is absent, then deterministic
   vacant state and first acquire produce one fence and complete receipt/head.
3. RED/GREEN: conflict, stale head, exact retry after advancement, changed
   command content, and cross-project isolation.
4. RED/GREEN: heartbeat/expiry/suspect/exact release/revoke/reacquire state
   matrix and no partial mutation on denial.
5. RED/GREEN: zero/max/overflow/rollback/non-reuse fence/epoch/revision matrix.
6. RED/GREEN: PID/start identity and replaced-daemon recovery evidence
   substitution matrix.
7. RED/GREEN: admission matrix and canonical time boundary.
8. RED/GREEN: untrusted aggregate replay/corruption/orphan/truncation/
   substitution matrix.
9. RED/GREEN: Policy consumes only exact owner receipt/current head and all
   prior Policy tests remain passing.
10. REVIEW RED/GREEN: every actionable independent finding receives a failing
    regression before repair.

## Verification

| Check | Command or service | Expected evidence |
|---|---|---|
| Shared contracts | `cargo test -p lattice-contracts --locked` | fixed owner, positive i64 values, receipt/head validation |
| Writer behavior | `cargo test -p lattice-writer-lease --locked` | transition/retry/recovery/replay/admission matrices |
| Policy composition | `cargo test -p lattice-policy --locked` | exact receipt/current-head sufficiency |
| V1 characterization | `node --test test/workspace-lock.test.js` | retained 9-test legacy oracle |
| Format | `cargo fmt --all -- --check` | exit 0 |
| Lint | `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings` | exit 0 |
| Full Rust suite | `cargo test --workspace --all-targets --all-features --locked` | exit 0 |
| Preserved Node suite | `npm.cmd run verify` | exit 0 |
| Dependencies | `cargo tree -p lattice-writer-lease --edges normal --locked` | only contracts/cjson/time approved edges |
| Scope/hygiene | forbidden-I/O scan plus `git diff --check` | zero forbidden source matches; exit 0 |

## Human Gate

None for this bounded pure/fake local implementation. The user's directive
authorizes continued local execution through MVP-3. Credentials/account/
payment actions, public exposure, irreversible actions, security-control
changes, real product effects, live/durable authority, protected release,
primary-branch merge, publication, and deployment remain outside this ticket.

## Completion Evidence — 2026-07-29

- Focused Writer Lease verification passes 2 unit plus 22 integration tests.
- Policy 2.5 verification passes 81 tests, including actual fake-owner
  current-head composition through `evaluate`.
- The locked full Rust workspace passes 180 tests.
- Strict locked workspace Clippy, Rust format, project governance validation,
  Cargo dependency checks, forbidden-I/O scans, V1 Writer Lock
  characterization, preserved Node verification, and `git diff --check` pass.
- Public raw parsing, receipt chaining, command high-water/tail commitment, and
  independently retained checkpoint comparison close all accepted
  code/security review findings with RED/GREEN regressions.
- Independent final code/security and architecture reviews return `PASS` with
  zero remaining P0, P1, P2, or P3 finding.
- SPEC-002 AC-28 is complete. AC-05 remains open exactly as required for
  PostgreSQL transaction, concurrency, database-clock, restart, and stale-live-
  connection evidence in Step 6.
- No database, provider, product repository, credential/account/payment,
  publication, deployment, protected release, commit, push, or merge action
  occurred.
