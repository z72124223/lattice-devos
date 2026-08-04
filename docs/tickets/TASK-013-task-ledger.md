---
ticket_id: TASK-013
spec_id: SPEC-002
spec_version: 9
module_id: task-ledger
constitution_version: 2.0
status: completed
parallel_safe: false
depends_on:
  - TASK-012
allowed_paths:
  - Cargo.toml
  - Cargo.lock
  - crates/lattice-contracts/**
  - crates/lattice-policy/**
  - crates/lattice-task-ledger/**
  - PLANS.md
  - HANDOFF.md
  - docs/specs/SPEC-002-autonomous-development-platform.md
  - docs/adr/ADR-009-policy-decision-facts-and-fail-closed-intents.md
  - docs/adr/ADR-011-task-ledger-event-receipt-and-resource-ownership.md
  - docs/modules/V2_AMENDMENT_PROPOSAL.md
  - docs/modules/README.md
  - docs/modules/lattice-contracts/**
  - docs/modules/policy-engine/**
  - docs/modules/task-ledger/**
  - docs/tickets/TASK-013-task-ledger.md
  - docs/workflow/WORKFLOW_LEDGER.md
  - docs/reviews/WORKFLOW_AUDIT_TASK_013_2026-07-29.md
  - docs/reviews/CODE_REVIEW_TASK_013_2026-07-29.md
  - docs/reviews/ARCHITECTURE_REVIEW_TASK_013_2026-07-29.md
  - docs/reviews/INTEGRATION_TASK_013_2026-07-29.md
likely_files:
  - Cargo.toml
  - Cargo.lock
  - crates/lattice-contracts/src/lib.rs
  - crates/lattice-contracts/tests/contracts.rs
  - crates/lattice-policy/src/types.rs
  - crates/lattice-policy/src/checks.rs
  - crates/lattice-policy/tests/policy_contract.rs
  - crates/lattice-policy/tests/policy_matrix.rs
  - crates/lattice-task-ledger/Cargo.toml
  - crates/lattice-task-ledger/src/lib.rs
  - crates/lattice-task-ledger/tests/task_ledger.rs
branch: feature/v2-rust-postgres-bootstrap
---

## Objective

Implement Task Ledger 2.0 as a pure Rust semantic owner with exact
domain-separated stream/event/head/request/receipt hashes, verified replay,
stable command idempotency, typed bounded diagnostics, Ledger-owned resource
projection, fixed-producer resource observations, and a deterministic
non-durable fake.

## Acceptance Criteria

- [x] A complete project/snapshot/task/revision/spec/currency identity produces
  one deterministic task stream ID and full zero head; invalid, non-NFC,
  whitespace-padded, NUL-bearing, unknown, or cross-stream identity rejects.
- [x] Append binds the complete expected head and every semantic event field.
  Sequence begins at one, predecessor/hash/resource projection are exact, and
  overflow fails without partial mutation.
- [x] Same stream/command/request returns the identical terminal receipt before
  stale-head evaluation, including after later appends. Changed content
  returns command-ID reuse; the same command ID in another stream is
  independent.
- [x] A new stale sequence, predecessor, stream, or full-head request returns a
  stable terminal denied receipt with no event/head/resource mutation.
- [x] Replay rejects unknown version/event, field/hash/sequence/predecessor/
  stream substitution, reorder, truncation, duplicate, orphan receipt, and
  claimed head/resource-projection disagreement.
- [x] One public untrusted persistence snapshot retains complete raw event and
  command-key/request/receipt rows, verifies appended and denied receipts, and
  returns only a typed reconstructed stream; Fake delegates to it.
- [x] Authoritative fields are typed. Optional diagnostics are bounded,
  non-authoritative, sanitized before hashing, and cannot leak recognized
  secrets through events, receipts, Debug, Display, or errors.
- [x] Resource snapshots derive only from verified events and cannot decrease
  cumulative counters or violate Implementer/agent bounds.
- [x] Contracts 1.3 fixes `lattice-task-ledger` producer version 2.0 and exposes
  immutable full Ledger head and resource observation receipt/head values.
- [x] Policy 2.4 removes caller-owned resource producer/owner/freshness fields,
  consumes the shared receipt plus independent current owner head, and denies
  every producer/runtime/project/task/spec/head/revision/claim/currency/
  counter/digest/historical substitution.
- [x] Task Ledger has no Task Domain, Policy, ports, store, filesystem,
  database, process, network, environment, random, credential, provider,
  payment, publication, deployment, or product-repository dependency/I/O.
- [x] AC-03 and durable/restart portions of AC-04 remain explicitly open;
  fake/runtime evidence never claims PostgreSQL truth or live effect authority.

## Non-Goals

- Create or connect to PostgreSQL, define SQL/migrations, run an outbox worker,
  or prove transaction, concurrency, unknown-commit, restart, or power-loss
  behavior.
- Decide Task Domain state-transition legality or build a Task Packet
  projection.
- Authenticate a producer/current-head lookup, daemon, epoch, runtime
  admission, approval, writer lease, or effect claimant.
- Import or approve a V1 compatibility manifest.
- Execute a provider, modify a product repository, install software, change
  credentials/accounts, pay, publish, push, merge, deploy, or activate a
  release.

## Module And Constitution Constraints

- Task Ledger 2.0 owns event/receipt/replay/resource meaning and depends only on
  Contracts 1.3, cjson mechanics, and exact `time` parsing/formatting.
- Contracts 1.3 owns neutral immutable representations, never mutable counters,
  issuance, hashing, freshness, persistence, or Policy meaning.
- Policy 2.4 owns only deterministic receipt/current-head sufficiency and has
  no normal/production Task Ledger dependency. Its TASK-013 integration test
  may use a one-way Ledger `dev-dependency` solely to obtain the fake owner's
  actual current head.
- Task Domain remains the only legal task-state owner; future Orchestrator
  composes both public contracts.

## Dependencies And Overlap

`parallel_safe: false`: this ticket changes shared Contracts and Policy public
types, the workspace lockfile, and a new owner crate. No other ticket may
change those paths or interfaces concurrently.

## TDD Behaviors

1. RED/GREEN: shared Ledger/resource receipt/head types are absent, then fixed
   producer/version/runtime and full-field equality reject substitution.
2. RED/GREEN: Task Ledger crate/API is absent, then exact stream identity and
   zero head are deterministic.
3. RED/GREEN: first append, second append, exact retry after advancement,
   cross-stream command reuse, and one-field request mutation.
4. RED/GREEN: stale sequence/hash/stream/full head and sequence overflow return
   stable denial without event/head/resource mutation.
5. RED/GREEN: domain/NFC/timestamp/identifier/diagnostic bounds and recognized
   secret redaction/rejection.
6. RED/GREEN: replay/tamper/reorder/truncate/duplicate/unknown/orphan/head/
   projection matrices fail closed.
7. RED/GREEN: resource snapshots, monotonic counters, observation receipts,
   historical invalidation, and full substitution matrices.
8. RED/GREEN: Policy consumes only the exact receipt/current head and all prior
   Policy tests remain passing.
9. REVIEW RED/GREEN: every actionable independent finding receives a failing
   regression before repair.

## Verification

| Check | Command or service | Expected evidence |
|---|---|---|
| Shared contracts | `cargo test -p lattice-contracts --locked` | fixed owner and receipt/head validation |
| Ledger behavior | `cargo test -p lattice-task-ledger --locked` | append/retry/replay/diagnostic/resource matrices |
| Policy composition | `cargo test -p lattice-policy --locked` | exact receipt/current-head sufficiency |
| Format | `cargo fmt --all -- --check` | exit 0 |
| Lint | `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings` | exit 0 |
| Full Rust suite | `cargo test --workspace --all-targets --all-features --locked` | exit 0 |
| Preserved Node suite | `npm.cmd run verify` | exit 0 |
| Dependencies | `cargo tree -p lattice-task-ledger --edges normal --locked` | only contracts/cjson/time approved edges |
| Scope/hygiene | forbidden-I/O scan plus `git diff --check` | zero forbidden source matches; exit 0 |

## Completion Evidence

- RED/GREEN implementation and adversarial review closed request/receipt/event
  substitution, cross-identity poisoning, corrupt retry, stale uncreated
  streams, diagnostic limits/leaks, Task ID parity, public terminal-denial
  export, and owner-currentness composition gaps.
- Contracts 13, Task Ledger 20, Policy 75, full Rust workspace 145, and
  preserved Node 38 tests pass.
- Format, locked workspace Clippy, selected constitution validation, normal
  dependency trees, explicit test-only dependency inspection, forbidden-I/O
  scan, project check, and diff hygiene pass.
- Independent final code/security and architecture reviews report `PASS` with
  zero remaining P0 through P3 findings.
- Local combined integration passes. Remote CI, committed-candidate evidence,
  branch protection, and primary-branch merge authorization remain missing or
  separately protected; no merge was performed.
- Review artifacts:
  `CODE_REVIEW_TASK_013_2026-07-29.md`,
  `ARCHITECTURE_REVIEW_TASK_013_2026-07-29.md`, and
  `INTEGRATION_TASK_013_2026-07-29.md`.

## Human Gate

None for this bounded pure/fake local implementation. The user's directive
authorizes continued local execution through MVP-3. PostgreSQL credentials and
mutation, account/payment actions, public exposure, irreversible actions,
security-control changes, real product effects, live/durable authority,
protected release activation, primary-branch merge, publication, and deployment
remain outside this ticket.
