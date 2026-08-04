---
ticket_id: TASK-012
spec_id: SPEC-002
spec_version: 8
module_id: project-registry
constitution_version: 1.1
status: completed
parallel_safe: false
depends_on:
  - TASK-011
allowed_paths:
  - Cargo.toml
  - Cargo.lock
  - crates/lattice-contracts/**
  - crates/lattice-task-domain/**
  - crates/lattice-policy/**
  - crates/lattice-project-registry/**
  - PLANS.md
  - HANDOFF.md
  - docs/specs/SPEC-002-autonomous-development-platform.md
  - docs/adr/ADR-009-policy-decision-facts-and-fail-closed-intents.md
  - docs/adr/ADR-010-project-registry-receipts-and-identity-ownership.md
  - docs/modules/V2_AMENDMENT_PROPOSAL.md
  - docs/modules/README.md
  - docs/modules/lattice-contracts/**
  - docs/modules/policy-engine/**
  - docs/modules/project-registry/**
  - docs/tickets/TASK-012-project-registry.md
  - docs/workflow/WORKFLOW_LEDGER.md
  - docs/reviews/WORKFLOW_AUDIT_TASK_012_2026-07-29.md
  - docs/reviews/CODE_REVIEW_TASK_012_2026-07-29.md
  - docs/reviews/ARCHITECTURE_REVIEW_TASK_012_2026-07-29.md
  - docs/reviews/INTEGRATION_TASK_012_2026-07-29.md
likely_files:
  - Cargo.toml
  - Cargo.lock
  - crates/lattice-contracts/src/lib.rs
  - crates/lattice-contracts/tests/contracts.rs
  - crates/lattice-task-domain/src/validation.rs
  - crates/lattice-policy/src/types.rs
  - crates/lattice-policy/src/checks.rs
  - crates/lattice-policy/src/evaluate.rs
  - crates/lattice-policy/tests/policy_contract.rs
  - crates/lattice-policy/tests/policy_matrix.rs
  - crates/lattice-project-registry/Cargo.toml
  - crates/lattice-project-registry/src/lib.rs
  - crates/lattice-project-registry/tests/project_registry.rs
branch: feature/v2-rust-postgres-bootstrap
---

## Objective

Implement the pure Rust Project Registry 1.1 owner boundary with deterministic
fake repository observations, immutable project snapshots, exact authority and
command receipts, accepted/pending identity reservations, distinct
zero-mutation denial and defensive blocking semantics,
drift/suspension/reconciliation, and the minimum shared identity values Policy
needs to consume without a direct Registry dependency.

## Acceptance Criteria

- [x] Shared contracts 1.2 validate one canonical `ProjectId`, preserve
  `ProjectClass`, fix the Registry producer ID/version, and construct only
  fully qualified local `refs/heads/*` `GitRefIdentity` values with a physical
  storage digest. Explicit pseudo-refs reject while valid uppercase branch
  names remain allowed.
- [x] A task-agnostic `ProjectAuthorityReceipt` binds producer/runtime,
  project/snapshot, non-zero Registry revision, lifecycle/class, primary ref,
  observation digest, and receipt digest; its full `ProjectAuthorityHead`
  mirrors every security field. `receipt.head()` is a structural projection,
  while currentness requires an independent owner lookup.
- [x] Registration returns revision/snapshot 1 and deterministic fake authority
  from an immutable observation containing canonical root, root identity,
  repository identity, file identity, and physical primary-ref identity.
  Command ID, canonical-root text, and primary-ref text must already be NFC.
- [x] Exact resolve reuses the current snapshot/head. Duplicate project,
  accepted root/repository/file identity, or pending reserved identity fails
  closed without mutation during registration/reconciliation, including
  aliases that carry different path text but the same physical identity.
- [x] The first non-colliding pending observation reserves its identities for
  the owning project. Another project cannot front-run it; a collision observer
  receives no second reservation.
- [x] Root move, repository/file replacement, primary-ref change, suspension,
  stale head, or cross-project substitution cannot return current active
  authority. An `ACTIVE` project's authoritative cross-project collision
  returns `Blocked`, advances its head to `SUSPENDED`, clears the colliding
  pending observation, and preserves the other project's reservation.
- [x] Exact reconciliation verifies project, old snapshot/head/revision,
  pending observation, decision kind, and evidence digest; it rotates
  revision/snapshot, preserves old receipts, and cannot change project class.
- [x] Every Registry command receipt binds command/request, before/after heads,
  terminal result, and result digest. Observe/suspend/reconcile also bind the
  expected full head; register has no prior head. Exact no-mutation observation
  still returns a receipt. Same command/same request replays identically; same
  command/different request rejects.
- [x] Policy 2.3 consumes the shared Registry receipt plus a head from an
  independent current owner lookup; it requires full producer/version,
  runtime, project/snapshot, revision, lifecycle/class, primary-ref,
  observation-digest, and receipt-digest equality and denies
  suspended/reconciliation/stale/substituted authority.
- [x] The future Scope Check composition requirement is documented with exact
  Registry receipt plus Task Spec/commit/diff/rule/report/revision bindings;
  TASK-012 does not claim Scope Check implementation.
- [x] The crate performs no filesystem, Git, database, process, network,
  clock, environment, credential, provider, payment, publication, deployment,
  or product-repository I/O.

## Non-Goals

- Inspect a real path, Windows file ID, junction, Git repository, loose/packed
  ref, commit, worktree, changed path, or conflict.
- Implement a Registry inspection port, PostgreSQL table/repository, daemon
  epoch gate, Scope Check, Workspace Git, orchestration, or a live owner.
- Authenticate a Registry producer, bind a receipt to a Task Spec inside
  Registry, or claim fake evidence is durable.
- Commit, push, merge, publish, deploy, install software, change credentials,
  or perform a protected action.

## Module And Constitution Constraints

- Project Registry 1.1 owns complete identity/lifecycle/receipt semantics and
  depends only on contracts 1.2 plus canonical-byte mechanics.
- `lattice-contracts` 1.2 owns shared representation, not durable/mutable
  project truth.
- Policy Engine 2.3 owns task-specific sufficiency/deny meaning and has no
  direct dependency on Registry.
- Task Domain retains Task Spec ownership and reuses the shared Project ID
  representation without acquiring Registry lifecycle authority.
- Workspace Git/Scope Check retain physical inspection, merge-readiness, and
  changed-path ownership.

## Dependencies And Overlap

`parallel_safe: false`: the ticket changes shared public identity/receipt
contracts, Policy consumption, the workspace lockfile, and a new owner module.
No other ticket may change those paths or interfaces concurrently.

## TDD Behaviors

1. RED/GREEN: shared Project ID, class, local-branch ref, physical identity,
   authority receipt, and head contracts are absent, then reject malformed and
   unsupported values.
2. RED/GREEN: Registry crate/API is absent, then registration issues a
   deterministic active revision/snapshot 1 receipt.
3. RED/GREEN: duplicate project/accepted/pending identity registration and
   reconciliation matrices deny without mutation; pending reservations resist
   front-running.
4. RED/GREEN: an authoritative cross-project observation cannot leave stale
   `ACTIVE` authority: it returns `Blocked`, rotates to `SUSPENDED`, creates no
   colliding reservation, lets the collision observer reactivate its prior
   accepted identity, and leaves the other project's accepted authority or
   pending reservation usable.
5. RED/GREEN: exact resolve reuses the head; move/replacement/ref drift rotates
   to reconciliation-required and cannot issue active authority.
6. RED/GREEN: suspension and exact move/identity/reactivation reconciliation
   enforce stale-head, decision, evidence, class, and snapshot-lineage rules.
7. RED/GREEN: same command/request replays the identical terminal receipt and
   command-ID subject substitution rejects.
8. RED/GREEN: non-NFC command/root/ref subjects, producer/version substitution,
   explicit pseudo-refs, and full-head field substitution reject; valid
   uppercase branches remain accepted.
9. RED/GREEN: Policy receipt/head/project/snapshot/runtime/lifecycle
   substitution matrices use an independent current head, deny all
   substitutions, and allow the exact retained fake Task Spec path.
10. REVIEW RED/GREEN: every actionable independent review finding receives a
   failing regression before repair.

## Verification

| Check | Command or service | Expected evidence |
|---|---|---|
| Shared contracts | `cargo test -p lattice-contracts --locked` | identity/receipt validation passes |
| Registry behavior | `cargo test -p lattice-project-registry --locked` | lifecycle, receipt, replay, isolation matrices pass |
| Policy composition | `cargo test -p lattice-policy --locked` | exact receipt/head and all prior matrices pass |
| Format | `cargo fmt --all -- --check` | exit 0 |
| Lint | `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings` | exit 0 |
| Full Rust suite | `cargo test --workspace --all-targets --all-features --locked` | exit 0 |
| Preserved Node suite | `npm.cmd run verify` | exit 0 |
| Dependencies | `cargo tree -p lattice-project-registry --edges normal --locked` | only contracts/cjson approved edges |
| Scope/hygiene | forbidden-I/O scan plus `git diff --check` | zero forbidden source matches; exit 0 |

## Human Gate

None for this bounded pure/fake local implementation. The user's directive
authorizes continued local execution through MVP-3. Credentials,
account/payment actions, public exposure, irreversible actions, security
control changes, real repository mutation, live/durable authority, protected
release activation, primary-branch merge, publication, and deployment remain
outside this ticket.
