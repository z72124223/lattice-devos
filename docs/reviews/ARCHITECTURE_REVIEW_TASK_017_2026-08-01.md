# TASK-017 Architecture Review

## Triggers

- New Gateway IPC wire-semantic owner, canonical codec, and bounded fake
  loopback.
- Material Contracts 1.7 request/reply representations and Ports 1.2 inbound
  `GatewayService` public contract.
- One Gateway, project isolation, peer-role authorization, exact retry,
  protected-release separation, and fake/live classification risk.
- New exact parser/NFC dependencies and cross-module ownership of neutral
  values versus wire semantics.

## Independent Result

`PASS`. P0=0, P1=0, P2=0, and P3=0. No additional architecture amendment or
local integration blocker is required.

Confirmed:

- OpenClaw remains the sole normal human gateway. The protocol exposes only
  Submit, Plan, Status, Approve, Reject, and exact task Stop; Recovery CLI can
  use only bounded Status and Stop and cannot submit normal work.
- Peer-role authorization precedes replay lookup. A narrower recovery peer
  cannot read an OpenClaw-cached result, and role denial cannot poison an
  authorized peer's replay key.
- Gateway IPC creates no second durable truth. Its bounded in-memory replay
  records are explicitly fake and non-durable; PostgreSQL remains the future
  command/event One Truth.
- Gateway IPC owns no product writer, lease, Codex process/thread, task-state
  transition, approval authority, or external effect. One Writer remains an
  Orchestrator, Writer Lease, PostgreSQL, and Codex composition responsibility.
- Replay identity binds project, server-derived actor, and command. Typed task
  subjects bind project, snapshot, task, revision, Task Spec digest, and owner
  evidence; project-status replies cannot contain another project's task.
- Fake peer context is supplied outside raw request bytes and always reports
  `RuntimeKind::Fake`. TASK-017 opens no listener, socket, Named Pipe, or live
  OpenClaw session and proves no OS authentication or durability.
- A normal request cannot represent protected-release authority. A raw
  `PROTECTED_RELEASE` label is rejected by the codec before service dispatch;
  normal `ProtectedChange` routing remains distinct and grants no approval.
- Contracts 1.7 owns neutral in-process representations, shared protocol
  identifiers, and constructor bounds. Gateway IPC 1.1 owns wire fields,
  canonical encoding, parser/encoded-frame limits, hash subjects, NFC
  preflight, and replay semantics. The ownership split is explicit and
  dependency direction remains acyclic.
- Ports 1.2 uses component-free `GatewayServiceError` for Rust-core routing
  failures. Core failures can no longer be falsely attributed to OpenClaw or
  another external adapter.
- Typed request/reply inputs must already be NFC. Allocation-free preflight
  rejects non-NFC and normalization-expanding values before hashing,
  normalized allocation, service dispatch, or replay insertion.
- `lattice-gateway-ipc` depends only on Contracts, Ports, cjson, exact
  `serde`/`serde_json`, exact `unicode-normalization`, and approved pure
  transitives. Ports depends only on Contracts.
- The crate performs no filesystem, Git, database, process, network,
  environment, credential, provider, payment, publication, deployment,
  release, or product-repository I/O. The unrelated playmate website is absent
  from active source and architecture.
- Project governance now machine-rejects duplicate ticket IDs and any
  `PLANS.md` state with other than one `CURRENT TASK-nnn` marker. The current
  tree contains one canonical TASK-017 and one current marker.

SPEC-002 v14, ADR-015, Gateway IPC constitution 1.1, Contracts constitution
1.7, Ports constitution 1.2, TASK-017, source, tests, and the module routing
index remain aligned; no further versioned constitution amendment is needed.

## Constitution Acceptance Gates

All Gateway IPC constitution gates have local evidence: canonical/adversarial
codec behavior; six closed actions and typed replies; Task Spec, session,
project/task/approval/stop binding; exact retry, substitution, bounded replay,
role and fault behavior; dependency/no-I/O absence; and full local verification.
Live transport, OS peer authentication, and PostgreSQL durability remain
explicitly deferred rather than misclassified as passed.

## Enforcement Truth

| Gate | Classification | Evidence |
|---|---|---|
| Closed wire protocol, canonical/NFC bounds, typed replies, role and replay behavior | machine-enforced locally | Rust types plus 31 Gateway IPC tests |
| Shared subject, project isolation, zero-sentinel, page, and protected-surface constraints | machine-enforced locally | 36 Contracts tests plus Gateway matrices |
| Truthful inbound service error and contracts-only Ports direction | machine-enforced locally | Rust type boundary plus 3 Ports tests |
| Dependency and no-I/O direction | machine-observed locally | Cargo tree, manifests, strict Clippy, and static scans |
| Unique ticket ID and exactly one current-task marker | machine-enforced locally | project check plus three isolated regression fixtures |
| Governance and ownership boundaries | documented plus structurally checked | SPEC v14, ADR-015, constitutions 1.1/1.7/1.2, TASK-017 |
| Live OpenClaw transport, ACL, peer identity, and session lifecycle | missing/deferred | MVP-2 exact-version compatibility ticket |
| PostgreSQL terminal receipts, durability, restart, and One Truth composition | missing/deferred | later MVP-1 store/orchestrator tickets |
| One Writer at composed runtime | documented-only in this slice | later Orchestrator, Writer Lease, and Codex integration |
| Remote Rust CI, branch protection, and merge authorization | missing/unverified | no remote/upstream or primary-merge authorization |

## Verification

- Focused Contracts, Gateway IPC, and Ports suites: 70/70 pass
  (36 Contracts, 31 Gateway IPC, and 3 Ports).
- `cargo test --workspace --all-targets --all-features --locked`: 358/358
  Rust tests pass.
- `npm.cmd run verify`: 41/41 Node tests pass, including the three project-
  governance regression fixtures.
- Strict locked workspace Clippy, format, dependency tree, forbidden-I/O/
  provider/product/unrelated-website scans, project check, and diff hygiene:
  pass.

## Residual Non-Blocking Owner Work

Exact-version live OpenClaw/plugin/schema compatibility, OS-local transport and
peer authentication, PostgreSQL terminal receipts/durability/restart,
Orchestrator composition, Codex One Writer execution, Graphify/Hermes/Codebase
Memory integration, and Guardian-protected release remain later bounded work.
Remote CI, branch protection, commit, primary-branch merge, publication, and
deployment are absent or unauthorized. TASK-017 makes no claim that these
exist.
