---
module_id: orchestrator-runtime
name: Orchestrator and Runtime Port
version: 2.1
status: active
owner: LATTICE maintainers
last_reviewed: 2026-08-05
---

## Mission

Provide the pure Rust effect coordinator that routes admitted requests and
orders policy, durable intent, runtime, workspace, verification, Git, terminal
evidence, review, stop, reconciliation, and improvement workflows only through
injected ports. `latticed`, not this module, is the application entry and
composition root.

## Non-Goals

- Maintain task truth outside Task Ledger/PostgreSQL.
- Parse raw OpenClaw frames or authenticate an IPC peer.
- Parse MCP stdio, select concrete adapters, load credentials/configuration, or
  provide a second application entry.
- Let an adapter append events, call another adapter, or own workflow order.
- Call PostgreSQL, a process, filesystem, test command, or Git implementation
  directly.
- Bypass Policy, approval, scope, writer lease, review, or release gates.

## Owned Data

- Command-to-workflow routing and call order.
- Transaction/effect intent and outcome coordination.
- Timeout, stop, cancellation, reconciliation, and daemon-epoch ordering.
- Pure scripted workflow scenarios and typed call-order state; no concrete
  adapter, transport, driver, process, filesystem, test, or Git state.

Task Domain owns Task Spec/state legality; Task Ledger/PostgreSQL owns durable
truth; Policy owns decisions; Gateway IPC owns protocol; Approval Verifier owns
approval authority; Writer Lease owns fencing; Guardian owns activation.

## Public Contracts

- Implement the typed `GatewayService` after codec/peer admission.
- Route submit/plan/status/normal approval/rejection/task-stop only.
- Revalidate Task Spec 2.1 through Task Domain before any task creation.
- Persist intent before an external effect and outcome after it.
- For typed delivery, call only injected ports in this order: durable intent,
  bounded workspace preparation, Codex, workspace changed-path inspection,
  fixed test, Git commit, durable terminal outcome/receipt. Codex is
  unreachable before workspace evidence, and Git is unreachable unless scope
  and test evidence pass.
- Load delivery status only through the injected delivery-ledger port.
- Stop on the first failed gate; ambiguous effects enter reconciliation.
- Revoke writer authority before verification/review and require a separate
  exact merge approval before integration.

## Invariants

1. Orchestrator is the only normal workflow event appender.
2. PostgreSQL remains the sole live durable command/event truth.
3. One current writer lease and one Codex process/thread owner exist per task.
4. Gateway routing cannot manufacture approval, writer, database, or Guardian
   authority.
5. `STOP_REQUESTED` precedes interrupt/reconciliation/lease release and is not
   terminal stop evidence.
6. First failed gate stops progress; unknown outcome never implies success.
7. Fake scenarios perform no model, network, credential, product, deployment,
   publication, or protected-release effect.
8. The crate has no concrete adapter, MCP, database driver, process, filesystem,
   test runner, or Git dependency.
9. The first failed or uncertain delivery stage stops later port calls;
   reconciliation cannot become success.
10. A compatibility caller reaches the same coordinator and cannot introduce a
    second call order or durable status source.

## Allowed Dependencies

- `lattice-contracts`, `lattice-task-domain`, `lattice-policy`, and
  `lattice-ports` public APIs.
- Injected Registry, Ledger, approval, workspace, runtime, verification,
  review, integration, clock, and ID ports.

## Forbidden Dependencies

- Direct OpenClaw SDK/raw IPC parser, direct provider/model clients, hidden
  filesystem/Git mutation, adapter-to-adapter calls, reviewer mutation ports,
  and direct Guardian trust-root access.
- MCP libraries/transports, concrete PostgreSQL/Codex/workspace/test/Git
  adapters, database/process/filesystem/Git clients, credentials, and product
  repositories.

## Failure, Compatibility, And Migration

Intent failure blocks an action. Unknown commit/effect outcome produces
reconciliation. Version 2.1 adds the typed TASK-032 delivery order while
removing application-entry ambiguity: `latticed` owns composition and this
module remains injected and I/O-free. Port/schema changes require compatibility
adapters and version evidence. The V1 Node fake remains characterization only.

## Acceptance Gates

| Gate | Evidence | Owner | Required for merge |
|---|---|---|---|
| Gateway routing | closed action and exact binding tests | Engineering | yes when implemented |
| Call order | intent/effect/outcome/stop/review tests | Engineering | yes when implemented |
| Delivery call order | intent -> workspace prepare -> Codex -> scope -> fixed test -> Git -> outcome/receipt, with first-failure call suppression | Engineering | yes |
| Fake scenarios | success/failure/timeout/cancel/malformed/ambiguous tests | Engineering | yes when implemented |
| No external call | injected-only dependency inspection | Security review | yes |
| End-to-end | one offline task with restart/replay evidence | MVP-1 exit | yes |

## Change Policy

Gate order, event ownership, writer authority, gateway routing, port contracts,
stop/reconciliation semantics, or agent limits require a versioned amendment,
SPEC/ADR update, architecture review, and responsible-user authorization.

## Amendment History

| Version | Date | Decision reference | Summary | Approver |
|---|---|---|---|---|
| 1.0 | 2026-07-29 | SPEC-001, ADR-001/002 | Initial Node deterministic control loop | Current user task |
| 2.0 | 2026-08-01 | SPEC-002 v13, ADR-004/005/006/007/015 | Rust routing, transaction/effect/stop/reconciliation boundary | User MVP-3 execution directive |
| 2.1 | 2026-08-05 | SPEC-002 v25, ADR-021, TASK-032 | Pure injected delivery ordering and explicit separation from `latticed` composition/MCP/concrete adapters | User approval in preceding implementation window |
