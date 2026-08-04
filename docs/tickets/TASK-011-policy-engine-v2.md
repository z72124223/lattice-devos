---
ticket_id: TASK-011
spec_id: SPEC-002
spec_version: 6
module_id: policy-engine
constitution_version: 2.1
status: completed
parallel_safe: false
depends_on:
  - TASK-010
allowed_paths:
  - Cargo.toml
  - Cargo.lock
  - crates/lattice-policy/**
  - crates/lattice-task-domain/**
  - PLANS.md
  - HANDOFF.md
  - docs/specs/SPEC-002-autonomous-development-platform.md
  - docs/adr/ADR-009-policy-decision-facts-and-fail-closed-intents.md
  - docs/adr/ADR-008-canonical-encoding-ownership.md
  - docs/modules/V2_AMENDMENT_PROPOSAL.md
  - docs/modules/README.md
  - docs/modules/policy-engine/**
  - docs/modules/task-domain/**
  - docs/tickets/TASK-011-policy-engine-v2.md
  - docs/workflow/WORKFLOW_LEDGER.md
  - docs/reviews/WORKFLOW_AUDIT_TASK_011_2026-07-29.md
  - docs/reviews/CODE_REVIEW_TASK_011_2026-07-29.md
  - docs/reviews/ARCHITECTURE_REVIEW_TASK_011_2026-07-29.md
  - docs/reviews/INTEGRATION_TASK_011_2026-07-29.md
likely_files:
  - Cargo.toml
  - Cargo.lock
  - crates/lattice-policy/Cargo.toml
  - crates/lattice-policy/src/lib.rs
  - crates/lattice-policy/src/decision.rs
  - crates/lattice-policy/src/evaluate.rs
  - crates/lattice-policy/src/types.rs
  - crates/lattice-policy/src/v1_compat.rs
  - crates/lattice-policy/tests/policy.rs
  - crates/lattice-task-domain/src/types.rs
  - crates/lattice-task-domain/src/spec.rs
  - crates/lattice-task-domain/tests/task_domain.rs
branch: feature/v2-rust-postgres-bootstrap
---

## Objective

Implement the pure Rust Policy Engine V2 as a deterministic, fail-closed
decision boundary over an immutable Task Spec and typed caller-supplied facts.
This slice freezes policy meaning before any database, approval-verifier,
writer-lease, gateway, or provider adapter may rely on it.

## Acceptance Criteria

- [x] Every public evaluation returns a typed `PolicyDecision`; absent Task
  Spec, unknown boundary value, incomplete subject, or mismatched immutable
  identity denies with a stable reason code.
- [x] A closed role/action/state matrix enforces One Writer: only Implementer
  can request product-code write or writable Codex execution; Integrator can
  perform an authorized non-conflicting metadata merge but cannot edit product
  files or repair conflicts.
- [x] The requested action requires its Task Spec capability. External lanes
  additionally require a current exact provider-capability fact bound to
  provider identity, version, executable digest, runtime/schema identity,
  contract version, project/snapshot, and Task Spec hash.
- [x] Risk floors combine with Task Spec execution/merge/protected-release
  requirements by selecting the stricter requirement. Primary merge requires
  exact Responsible User authority; protected release requires exact Guardian
  authority plus independent security and architecture evidence.
- [x] Approval subjects are domain-separated and typed. Merge binds Registry
  canonical primary identity and a fully qualified `refs/heads/*` target
  (never pseudo-ref, DWIM, tag, remote, or shorthand), reviewed commit, diff,
  target head, and Registry/Workspace-Git physical ref-identity digests. Policy
  classifies primary by the owner-produced identity, not platform-specific
  case folding, and
  fresh workspace-git/scope analysis. External cost binds
  amount/currency/provider/quote/pricing. Protected change binds class and
  operation. Release binds manifest/source/binary/migration/delta,
  slots/saga/epoch, and the same guardian runtime identity carried by the
  protected-release approval.
- [x] Normal mutation requires `ACTIVE` runtime admission. `DRAINING`,
  `CANARY`, `STOPPED`, and `RECONCILIATION_REQUIRED` permit only their exact
  bounded stop/release/health/rollback recovery actions. Runtime
  reconciliation uses a dedicated typed gate and separates normal
  daemon/effect recovery from guardian release-saga recovery. Every lane binds
  a typed resolved outcome and immutable resolution-evidence digest. Normal
  Runtime Supervisor recovery may move only to `STOPPED`; restoring `ACTIVE`
  requires the exact Guardian producer plus consistent durable saga,
  database, and boot-state evidence.
- [x] Product-code execution requires an exact current writer fact with one
  Implementer, matching lease holder, worktree, daemon instance/epoch, and
  positive non-wrapping fencing token. Normal writer release additionally
  binds the requesting actor to that lease holder.
- [x] Checked resource accounting consumes a fresh Task-Ledger-owned fact bound
  to the exact project/snapshot/task/revision/Task Spec, observation revision,
  claim identity, and one accounting currency. Every consuming gate carries
  an independent exact Ledger stream/head/revision/claim subject, so a valid
  same-task fact cannot be replayed for another effect. It denies missing, stale,
  mixed-currency, malformed, over-budget, or overflowed agents, duration,
  attempts, model calls, and decimal external cost; no floating-point value is
  used. The future effect owner must re-check and claim the counters in the
  same PostgreSQL transaction.
- [x] Allowlisted external networking denies until an immutable allowlist fact
  is bound. Authorized deployment produces policy intent only. Unknown or new
  external cost denies without Responsible User authority.
- [x] Memory and improvement candidates never create authority. Protected
  upgrade evaluation cannot accept normal policy/user-task authority, and the
  first A/B activation denies any schema migration. Rollback is a
  stage-specific subject that identifies the failed activation, reverses its
  source/target slots, and requests an epoch strictly greater than the failed
  activation epoch. The rollback carries the exact typed protected-release
  receipt for that failed activation, not an unchecked opaque digest.
- [x] Generic AgentAction cannot authorize merge, memory promotion, protected
  change, or upgrade lifecycle work. Recovery admits no newly requested
  network, deployment, agents, model calls, or external cost; guardian canary
  is limited to the LATTICE system project and reserved system stream.
- [x] Safe retained V1 role/state/lease denials and reason ordering remain
  namespaced characterization evidence. Missing-spec equality, unbound merge,
  risk/capability omission, fake-only ceilings, replayable nonce assumptions,
  and project-specific V1 actions are explicit V2 denial regressions.
- [x] The crate depends only on Rust standard library,
  `lattice-task-domain`, and `lattice-contracts`, and contains no I/O,
  persistence, clock, environment, process, network, credential, model,
  payment, publication, deployment, or product-repository implementation.

## Non-Goals

- Authenticate identities, verify signatures/MACs, consume a nonce, acquire or
  persist a writer lease, read a clock, register a project, or mutate runtime
  admission.
- Implement PostgreSQL, artifacts, local IPC, orchestration, Git worktrees,
  scope scanning, Codebase Memory persistence, guardian activation, or any
  fake/live external adapter.
- Turn an allow decision into an effect or claim real authority from a
  caller-supplied fact.
- Approve a V1 compatibility manifest or retain project-specific V1 behavior.

## Module And Constitution Constraints

- `policy-engine` 2.1 owns deterministic decision meaning and stable reason
  precedence only.
- Task Domain 2.1 owns immutable Task Spec, state, risk, capability, network,
  deployment, approval-requirement, and check types.
- `lattice-contracts` 1.0 supplies shared immutable identifiers and SHA-256
  references.
- Authority-producing modules must later authenticate, persist, refresh, and
  atomically claim/consume the facts Policy merely evaluates.

## Fixed Decision Precedence

1. Invalid or incomplete input.
2. Project and snapshot binding.
3. Runtime admission.
4. Role/action compatibility.
5. Task state.
6. Protected-subject routing.
7. Requested Task Spec capability.
8. Current provider capability.
9. Network, deployment, and external-cost envelope.
10. Risk and exact-subject approval.
11. Writer lease, daemon epoch, and fencing.
12. Checked resource budget.
13. Allow.

## TDD Behaviors

1. RED: contract tests fail because `lattice-policy` and its decision types do
   not exist. GREEN: complete valid input allows while absent/unknown input
   returns stable denials.
2. RED: exhaustive role/action/state tests expose over-broad permissions.
   GREEN: the closed matrices and fixed precedence pass.
3. RED: project, capability, network, deployment, cost, approval, writer, and
   resource substitution matrices expose any unbound fact. GREEN: exact
   matching and checked limits pass.
4. RED: runtime/memory/upgrade and retained V1 vulnerability tests expose
   authority bypasses. GREEN: only exact recovery/protected paths pass.
5. REVIEW RED/GREEN: every actionable independent review finding receives a
   failing regression test before repair, including Git-ref aliases,
   merge-readiness substitution, resource-fact replay/currency substitution,
   writer-release actor substitution, unscoped reconciliation,
   guardian-runtime substitution, and rollback direction/epoch substitution.
6. ARCHITECTURE REVIEW RED/GREEN: a requested recovery target without a typed
   resolved outcome allowed `RECONCILIATION_REQUIRED -> ACTIVE`. GREEN: normal
   recovery is stop-only; exact Guardian recovery binds its producer and
   durable saga/database/boot resolution before restoring `ACTIVE`.
7. SECURITY/CODE REVIEW RED/GREEN: `HEAD`/ambiguous Git namespaces, same-task
   resource-claim replay, unchecked rollback activation digests, and
   TaskDomain/Policy decimal-bound drift each reproduce before repair. GREEN:
   exact canonical branch, independent observation subject, typed failed
   activation receipt, and one shared 256-byte/127-integer/128-fractional
   decimal bound deny substitution and mixed-scale overflow.
8. FINAL SECURITY RED/GREEN: a Windows case-only ref alias resolves to the
   physical primary ref but was classified as feature by string comparison.
   GREEN: exact Registry/Workspace-Git physical ref identities classify it as
   primary and reject a feature declaration.

## Verification

| Check | Command or service | Expected evidence |
|---|---|---|
| Policy behavior | `cargo test -p lattice-policy --locked` | all decision and regression matrices pass |
| Format | `cargo fmt --check` | exit 0 |
| Lint | `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings` | exit 0 |
| Full Rust suite | `cargo test --workspace --locked` | exit 0 |
| Preserved Node suite | `npm.cmd run verify` | exit 0 |
| Dependencies | `cargo metadata --locked --format-version 1` plus source scan | only approved crate edges and no I/O dependency |
| Scope/hygiene | exact allowed-path audit plus `git diff --check` | zero TASK-011 foreign change; exit 0 |

## Human Gate

None for this bounded local implementation. Typed authority facts are test
inputs, not live authentication evidence. Credentials, account/payment
actions, public exposure, irreversible actions, live providers, protected
release activation, primary-branch merge, publication, and deployment remain
outside this ticket.

## Completion Evidence

- Initial API tests failed before `lattice-policy` existed, then passed after
  the pure decision contracts and evaluator were implemented.
- Independent review RED/GREEN covers Guardian substitution, exact merge
  readiness/conflict/target head, same-task resource-claim replay, currency and
  decimal-bound drift, writer actor substitution, generic reconciliation,
  typed durable recovery, rollback receipt/direction/epoch, Git
  pseudo-ref/DWIM, and Windows case-only physical-ref aliases.
- Policy: 66 tests pass; Task Domain: 6 tests pass; full Rust workspace:
  94 tests pass; preserved Node suite: 38 tests pass.
- Formatting, locked all-target/all-feature Clippy with `-D warnings`, Cargo
  metadata/tree, forbidden-I/O scan, project check, and `git diff --check`
  pass.
- Independent code, security, and architecture reviews all return `PASS`.
- Local combined integration passes. Remote CI and merge readiness remain
  separately blocked because there is no committed candidate, remote, branch
  policy, or primary-merge authorization.
