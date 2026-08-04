---
ticket_id: TASK-017
spec_id: SPEC-002
spec_version: 14
module_id: gateway-ipc
constitution_version: 1.1
status: completed
parallel_safe: false
depends_on:
  - TASK-016
allowed_paths:
  - Cargo.toml
  - Cargo.lock
  - crates/lattice-contracts/**
  - crates/lattice-ports/**
  - crates/lattice-gateway-ipc/**
  - scripts/check-project.mjs
  - test/project-governance-check.test.js
  - PLANS.md
  - HANDOFF.md
  - docs/PROJECT_CHARTER.md
  - docs/specs/SPEC-002-autonomous-development-platform.md
  - docs/adr/ADR-015-versioned-local-gateway-ipc-boundary.md
  - docs/modules/V2_AMENDMENT_PROPOSAL.md
  - docs/modules/README.md
  - docs/modules/gateway-ipc/**
  - docs/modules/lattice-contracts/**
  - docs/modules/lattice-ports/**
  - docs/modules/openclaw-adapter/**
  - docs/modules/orchestrator-runtime/**
  - docs/tickets/TASK-017-gateway-ipc.md
  - docs/workflow/WORKFLOW_LEDGER.md
  - docs/reviews/WORKFLOW_AUDIT_TASK_017_2026-08-01.md
  - docs/reviews/CODE_REVIEW_TASK_017_2026-08-01.md
  - docs/reviews/ARCHITECTURE_REVIEW_TASK_017_2026-08-01.md
  - docs/reviews/INTEGRATION_TASK_017_2026-08-01.md
likely_files:
  - Cargo.toml
  - Cargo.lock
  - crates/lattice-contracts/src/lib.rs
  - crates/lattice-contracts/tests/contracts.rs
  - crates/lattice-ports/src/lib.rs
  - crates/lattice-ports/tests/ports.rs
  - crates/lattice-gateway-ipc/Cargo.toml
  - crates/lattice-gateway-ipc/src/lib.rs
  - crates/lattice-gateway-ipc/tests/gateway_ipc.rs
  - scripts/check-project.mjs
  - test/project-governance-check.test.js
branch: feature/v2-rust-postgres-bootstrap
---

## Objective

Implement Gateway IPC module 1.1 and wire protocol 1.0 as a pure, versioned,
canonical, bounded local protocol plus deterministic in-memory loopback fake
for typed Submit, Plan, Status, normal Approve/Reject, and task Stop routing.
Preserve Task Domain ownership, `GatewayService` in `lattice-ports`, One
Gateway/One Truth/One Writer, AC-24 dependency direction, and explicit
fake-only evidence.

## Acceptance Criteria

- [x] Contracts 1.7 provides neutral immutable protocol version, out-of-band
  fake peer context, command/correlation identity, action-specific request,
  exact subject, typed reply/disposition, stable denial, and digest values.
- [x] Ports 1.2 keeps `GatewayService` in `lattice-ports`, changes it to accept
  peer context plus typed request and return a typed reply/component-free core
  error, and retains the contracts-only dependency.
- [x] Gateway IPC 1.1 owns wire field selection, canonical frames,
  request/reply digest subjects, parser/encoded-frame bounds, and fake retry;
  Contracts 1.7 owns neutral representation and constructor-level bounds.
- [x] The complete raw canonical JSON frame has an exact `1_048_576` byte
  maximum. Exact-bound input reaches structural parsing; one byte over denies
  before parse, Unicode normalization, content-proportional allocation,
  hashing, or service dispatch.
- [x] Non-UTF-8, duplicate keys, unknown fields/actions/versions, malformed or
  non-canonical values, excessive nesting, truncation, trailing data, and
  request/reply shape disagreement fail closed with stable bounded redacted
  errors.
- [x] Typed encoder/request/reply inputs must already be NFC; non-NFC and
  normalization-expanding identifiers fail before canonical hashing,
  normalized allocation, replay insertion, or service dispatch.
- [x] Submit carries only fixed `lattice.task-spec`/`2.1`, a bounded canonical
  JSON document, and claimed SHA-256 digest. IPC does not interpret Task Spec
  semantics, and the raw document never enters `Debug`, errors, receipts, or
  replies.
- [x] The composed-service contract requires Task Domain to validate and
  recompute the Task Spec digest before task creation; TASK-017 tests the
  carrier and mismatch route without claiming a complete Orchestrator flow.
- [x] The request set is exactly Submit, Plan, Status, Approve, Reject, and task
  Stop. No generic action/payload, SQL, shell, provider call, credential,
  arbitrary product path, daemon control, merge, or release command exists.
- [x] Task-scoped requests bind exact project, snapshot, task, revision, and
  spec digest. Cross-project/snapshot/task/subject substitution denies without
  leaking status or changing fake state.
- [x] Approve/Reject routes an exact normal approval challenge/subject only.
  Fake gateway evidence, a normal peer/session, and a normal approval cannot
  represent or satisfy protected-release authority.
- [x] Status returns a typed bounded projection backed by owner-evidence
  digests; the IPC fake keeps no independent mutable task truth.
- [x] Stop returns exactly requested, already-terminal, or reconciliation-
  required disposition and never converts routing or an unknown outcome into
  a completed-interrupt claim.
- [x] Reply variants are action-specific submit accepted, plan routed, status
  observed, approval/rejection routed, stop routed, denied, and unknown
  outcome. They bind protocol version, command/correlation ID, action, request
  digest, and exact subject; no bare success Boolean or arbitrary text exists.
- [x] Exact retry under one command ID and identical canonical request returns
  the identical terminal reply before mutable observations. Changed content
  under the same command ID denies permanently with zero partial fake change.
- [x] Peer context is supplied outside request bytes. TASK-017 can construct
  only visibly fake peer/runtime evidence; caller-controlled fields cannot
  claim live authentication or Approval Verifier authority.
- [x] The deterministic loopback fake covers success, denial, exact retry,
  changed command, unavailable, version mismatch, malformed, timeout,
  cancellation, and ambiguous/unknown outcome without opening a listener.
- [x] The crate performs no filesystem, database, Git, process, network,
  environment, provider, credential, model, payment, product, publication,
  deployment, or release I/O and does not install or invoke OpenClaw.
- [x] `lattice-gateway-ipc` depends only on Contracts 1.7, Ports 1.2, cjson,
  exact `serde`/`serde_json` parser dependencies, exact
  `unicode-normalization` NFC preflight, and the Rust standard library. It does
  not depend on Orchestrator, Task Domain, Policy, adapters, or product code;
  no adapter-to-adapter edge exists.
- [x] CLI source, commands, dependencies, and constitution remain unchanged in
  TASK-017.
- [x] Project check rejects duplicate ticket IDs and any `PLANS.md` state with
  other than one current-task marker; regression fixtures prove both failures
  and one valid case without changing product data.
- [x] AC-31 completion was gated on implementation, focused/full verification,
  independent reviews, and local integration; all passed. AC-07 live OpenClaw
  transport/authentication remains open for MVP-2.

## Non-Goals

- Install, load, invoke, authenticate, or capability-probe OpenClaw.
- Select or open Named Pipe, Unix socket, TCP, HTTP, or any listener.
- Implement OS peer authentication, endpoint ACL, credentials, session/token
  storage/rotation/revocation, or live approval authority.
- Implement Orchestrator task behavior, Task Domain parsing changes,
  PostgreSQL, Git/workspace, provider, model, product, or filesystem effects.
- Add or modify CLI operational commands.
- Claim live compatibility, durability, One Gateway composition, full AC-07,
  MVP-1 completion, merge readiness, release, publication, or deployment.
- Commit, push, merge, deploy, or activate a release.

## Module And Constitution Constraints

- Gateway IPC 1.1 owns wire semantics only, not Contracts constructor bounds,
  Task Spec, task state, Policy, approval, Orchestrator, persistence, or
  transport authentication.
- Contracts 1.7 represents values only and remains serialization/hash/I/O
  free.
- Ports 1.2 remains contracts-only; `GatewayService` stays inbound, uses a
  component-free core error, and does not select a transport or adapter.
- OpenClaw Adapter 2.0 remains a thin future client; TASK-017 neither installs
  nor invokes it.
- Orchestrator Runtime 2.0 remains the future service implementation owner;
  the fake accepts an injected service and does not duplicate orchestration.

## Dependencies And Overlap

`parallel_safe: false`: this ticket changes shared Contracts and Ports public
types plus the workspace manifest/lockfile. No concurrent ticket may change
those paths, gateway IPC schema, or `GatewayService`.

## TDD Behaviors

1. RED/GREEN: protocol values and typed `GatewayService` are absent, then all
   valid values construct and every empty/unknown/cross-labeled value denies.
2. RED/GREEN: raw 1 MiB canonical frame reaches parse, one byte over is denied
   before parse/dispatch, and control-character expansion cannot bypass the
   canonical encoded-size bound.
3. RED/GREEN: canonical request/reply golden bytes round-trip exactly; malformed,
   duplicate, unknown, non-canonical, deep, truncated, and trailing input deny.
4. RED/GREEN: bounded Task Spec 2.1 canonical document plus claimed digest
   round-trips with raw-byte redaction and mismatch denial route.
5. RED/GREEN: all six request variants produce only their corresponding typed
   reply variants and reject project/subject/action substitution.
6. RED/GREEN: normal approval routing rejects protected subjects, trust-lane
   substitution, fake/live substitution, and self-created authority.
7. RED/GREEN: exact retry returns byte-identical terminal reply; changed
   command and unknown outcome preserve zero-partial-change/reconciliation.
8. RED/GREEN: fake client/server covers unavailable, timeout, cancel, malformed,
   version mismatch, duplicate delivery, and ambiguous outcomes without I/O.
9. REVIEW RED/GREEN: every accepted independent finding receives a failing
   regression before repair.

## Verification

| Check | Command or service | Expected evidence |
|---|---|---|
| Shared contracts | `cargo test -p lattice-contracts --locked` | complete gateway values and substitution matrix |
| Port contract | `cargo test -p lattice-ports --locked` | typed service and contracts-only behavior |
| Gateway IPC | `cargo test -p lattice-gateway-ipc --locked` | codec/bounds/actions/replies/retry/failure matrices |
| Format | `cargo fmt --all -- --check` | exit 0 |
| Lint | `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings` | exit 0 |
| Full Rust suite | `cargo test --workspace --all-targets --all-features --locked` | exit 0 |
| Preserved Node suite | `npm.cmd run verify` | exit 0 |
| Dependencies | Cargo metadata/tree plus architecture scan | ports only contracts; IPC only approved edges |
| Fake-only scope | listener/I/O/auth/install/provider/product/CLI scans | zero forbidden source or dependency matches |
| Diff hygiene | `git diff --check` | exit 0 |

## Human Gate

None for this bounded pure/fake local implementation. The approved V2 module
direction and direct MVP-3 execution instruction authorize the versioned local
amendments and tests. Credentials/accounts/payment, public exposure/
publication/deployment, irreversible deletion or migration, security-control
disablement, protected release, and primary-branch merge remain separately
protected. Live OpenClaw transport/authentication remains a later MVP-2 gate.
