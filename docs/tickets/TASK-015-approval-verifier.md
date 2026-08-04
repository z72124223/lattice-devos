---
ticket_id: TASK-015
spec_id: SPEC-002
spec_version: 11
module_id: approval-verifier
constitution_version: 1.0
status: completed
parallel_safe: false
depends_on:
  - TASK-014
allowed_paths:
  - Cargo.toml
  - Cargo.lock
  - crates/lattice-contracts/**
  - crates/lattice-policy/**
  - crates/lattice-approval-verifier/**
  - PLANS.md
  - HANDOFF.md
  - docs/specs/SPEC-002-autonomous-development-platform.md
  - docs/adr/ADR-007-guarded-self-improvement-and-upgrade.md
  - docs/adr/ADR-008-canonical-encoding-ownership.md
  - docs/adr/ADR-009-policy-decision-facts-and-fail-closed-intents.md
  - docs/adr/ADR-013-approval-subject-proof-currentness-and-nonce-claim.md
  - docs/modules/V2_AMENDMENT_PROPOSAL.md
  - docs/modules/README.md
  - docs/modules/lattice-contracts/**
  - docs/modules/policy-engine/**
  - docs/modules/approval-verifier/**
  - docs/tickets/TASK-015-approval-verifier.md
  - docs/workflow/WORKFLOW_LEDGER.md
  - docs/reviews/WORKFLOW_AUDIT_TASK_015_2026-07-29.md
  - docs/reviews/CODE_REVIEW_TASK_015_2026-07-29.md
  - docs/reviews/ARCHITECTURE_REVIEW_TASK_015_2026-07-29.md
  - docs/reviews/INTEGRATION_TASK_015_2026-07-29.md
likely_files:
  - Cargo.toml
  - Cargo.lock
  - crates/lattice-contracts/src/lib.rs
  - crates/lattice-contracts/tests/contracts.rs
  - crates/lattice-policy/Cargo.toml
  - crates/lattice-policy/src/types.rs
  - crates/lattice-policy/src/checks.rs
  - crates/lattice-policy/src/decision.rs
  - crates/lattice-policy/tests/policy_matrix.rs
  - crates/lattice-approval-verifier/Cargo.toml
  - crates/lattice-approval-verifier/src/lib.rs
  - crates/lattice-approval-verifier/tests/approval_verifier.rs
branch: feature/v2-rust-postgres-bootstrap
---

## Objective

Implement Approval Verifier 1.0 as the pure owner of complete typed approval
subjects, challenge/proof/nonce/time semantics, exact command idempotency,
current-head availability, raw aggregate replay, and deterministic fake
composition. Replace Policy's caller-owned approval/review verdict Booleans
with owner receipt/current-head evidence and fail closed for R3 until Review
Runtime has its own owner contract.

## Acceptance Criteria

- [x] Contracts 1.5 provides the complete neutral typed approval subject graph,
  fixed `lattice-approval-verifier`/`1.0` producer values, runtime, positive
  revision, identity, availability, receipt, and complete authority head.
- [x] Approval kind derives from the typed subject; contradictory kind/subject
  combinations are unrepresentable. Protected release can only use the
  Guardian trust lane.
- [x] Challenge and receipt hashes bind complete binding/subject, requester,
  approver, authority/trust, channel/session, nonce commitment, issue/expiry,
  runtime, authenticator/key identity, proof/evidence, review-set, and receipt
  digests.
- [x] Raw nonce/token/key/assertion bytes never enter stored requests,
  aggregates, snapshots, errors, or `Debug`; fake evidence remains visibly
  fake.
- [x] One nonce commitment permanently binds one approval/challenge/subject/
  lane. Cross-approval, cross-project, cross-subject, or cross-lane reuse
  rejects without partial authority mutation.
- [x] Exact command retry returns the identical applied or denied terminal
  receipt before stale/time checks. Changed content under one command ID
  rejects permanently.
- [x] Canonical time enforces `issued_at < expires_at` and
  `issued_at <= observed_at < expires_at`; equality with expiry denies.
- [x] `current_head_at` returns an available independent head only for an exact
  verified, unclaimed, unrevoked, unexpired approval.
- [x] Revocation is an exact-head terminal transition only for verified
  available normal or protected-pending authority. It requires an observation
  inside the approval validity interval, the original approver as revoker, and
  a non-zero evidence digest; advances revision; emits a typed immutable
  revocation; removes current-head availability; and is covered by exact
  retry, raw replay, and trusted checkpoint verification. Fake evidence proves
  binding only; live normal/protected evidence authentication belongs to the
  OS/Guardian trust adapter respectively. Approval Verifier 1.0 has no other
  revoker or override.
- [x] Normal claim planning invalidates later current-head lookup. No public
  general protected-release consume command exists.
- [x] Public raw replay rejects unknown versions/kinds, malformed values,
  tamper, reorder, truncation, duplication, orphan rows, nonce rebinding,
  fake/live mixing, receipt-chain/high-water disagreement, and claimed-state
  mismatch.
- [x] Rollback-sensitive restore requires an independently retained validated
  checkpoint and rejects a coherent older prefix.
- [x] Policy 2.6 removes `subject_verified`, `identity_verified`, `fresh`,
  `nonce_available`, `self_approved`, and caller `ReviewChecks`; it compares
  complete expected typed subject with owner receipt and an independently
  queried current head.
- [x] Policy denies missing/historical/claimed/expired/revoked/substituted/
  wrong-lane/fake-live approval. Positive tests use an actual
  `FakeApprovalVerifier` receipt/current-head pair.
- [x] R3 and every `require_independent_checks` allow path fail closed with
  explicit missing Review Runtime authority; Approval Verifier does not
  manufacture review authority.
- [x] Approval Verifier has no Policy, Task Domain, Ledger, Registry, Writer,
  ports/store, filesystem, database, process, network, environment, random,
  credential, provider, payment, publication, deployment, Guardian, or
  product-repository dependency/I/O.
- [x] Live OS authentication/cryptography, database uniqueness/clock/
  durability/restart/atomic claim, OpenClaw IPC, Review Runtime, and Guardian
  activation remain explicitly open.

## Non-Goals

- Install or invoke OpenClaw, a model, cryptographic provider, database, or OS
  authentication UI.
- Store a real key, token, credential, nonce secret, or raw authentication
  assertion.
- Define PostgreSQL migrations, roles, tables, indexes, or live transactions.
- Persist/perform a task transition, effect, merge, memory promotion, release,
  publication, or deployment.
- Implement Review Runtime or claim that R3 has passed.
- Commit, push, merge, or activate a protected release.

## Module And Constitution Constraints

- Approval Verifier 1.0 owns pure subject/challenge/proof/nonce/currentness
  semantics and depends only on Contracts 1.5, cjson mechanics, and exact time
  parsing/formatting.
- Contracts 1.5 owns immutable representation only.
- Policy 2.6 has no normal Approval Verifier dependency. A test-only dependency
  may obtain a real fake-owner receipt/current head.
- Future store/Guardian composition performs actual atomic claims but may not
  duplicate verifier state or hash semantics.

## Dependencies And Overlap

`parallel_safe: false`: this ticket materially changes shared Contracts and
Policy public types and the workspace lockfile. No other ticket may modify
these paths or interfaces concurrently.

## TDD Behaviors

1. RED/GREEN: shared complete subject/receipt/head values are absent, then
   reject every producer/version/runtime/identity/subject/head substitution.
2. RED/GREEN: verifier crate/fake is absent, then issue one deterministic
   challenge and verify one exact fake proof.
3. RED/GREEN: global nonce binding, cross-subject/lane reuse, exact retry,
   changed command, and stale expected head.
4. RED/GREEN: requester/approver self identity, actor/channel/session,
   authority/trust root, fake/live, proof/evidence substitution.
5. RED/GREEN: before-issued, valid interval, expiry equality/after, malformed
   and reversed time.
6. RED/GREEN: normal claim invalidation, protected no-consume boundary, and
   exact original-approver normal/protected revocation with typed terminal
   record, wrong-state/actor/time denial, current-head loss, retry, and replay.
7. RED/GREEN: raw aggregate corruption, denied-tail truncation, and coherent
   rollback against a trusted checkpoint.
8. RED/GREEN: Policy uses actual fake-owner currentness, with complete
   receipt/head/expected-subject substitution matrices.
9. RED/GREEN: caller review Booleans disappear and R3 denies pending Review
   Runtime owner authority.
10. REVIEW RED/GREEN: every actionable independent finding receives a failing
    regression before repair.

## Verification

| Check | Command or service | Expected evidence |
|---|---|---|
| Shared contracts | `cargo test -p lattice-contracts --locked` | full typed approval subject and receipt/head validation |
| Approval behavior | `cargo test -p lattice-approval-verifier --locked` | challenge/proof/nonce/time/retry/replay/checkpoint matrices |
| Policy composition | `cargo test -p lattice-policy --locked` | exact receipt/current-head sufficiency and R3 fail closed |
| Format | `cargo fmt --all -- --check` | exit 0 |
| Lint | `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings` | exit 0 |
| Full Rust suite | `cargo test --workspace --all-targets --all-features --locked` | exit 0 |
| Preserved Node suite | `npm.cmd run verify` | exit 0 |
| Dependencies | `cargo tree -p lattice-approval-verifier --edges normal --locked` | only contracts/cjson/time approved edges |
| Secrets/scope | forbidden-I/O and raw-secret scans plus `git diff --check` | zero forbidden source matches; exit 0 |

## Human Gate

None for this bounded pure/fake local implementation. Credentials/account/
payment actions, public exposure, irreversible actions, security-control
changes, live trust roots, real product effects, protected release, primary-
branch merge, publication, and deployment remain outside this ticket.
