---
module_id: orchestrator-runtime
name: Orchestrator and Runtime Port
version: 2.0
status: active
owner: LATTICE maintainers
last_reviewed: 2026-08-01
---

## Mission

Provide the single Rust application entry that routes admitted gateway
requests and orders policy, durable intent, workspace, runtime, verification,
review, integration, stop, reconciliation, and improvement workflows.

## Non-Goals

- Maintain task truth outside Task Ledger/PostgreSQL.
- Parse raw OpenClaw frames or authenticate an IPC peer.
- Let an adapter append events, call another adapter, or own workflow order.
- Bypass Policy, approval, scope, writer lease, review, or release gates.

## Owned Data

- Command-to-workflow routing and call order.
- Transaction/effect intent and outcome coordination.
- Timeout, stop, cancellation, reconciliation, and daemon-epoch ordering.
- Fake scripted workflow scenarios until concrete adapters are introduced.

Task Domain owns Task Spec/state legality; Task Ledger/PostgreSQL owns durable
truth; Policy owns decisions; Gateway IPC owns protocol; Approval Verifier owns
approval authority; Writer Lease owns fencing; Guardian owns activation.

## Public Contracts

- Implement the typed `GatewayService` after codec/peer admission.
- Route submit/plan/status/normal approval/rejection/task-stop only.
- Revalidate Task Spec 2.1 through Task Domain before any task creation.
- Persist intent before an external effect and outcome after it.
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

## Allowed Dependencies

- `lattice-contracts`, `lattice-task-domain`, `lattice-policy`, and
  `lattice-ports` public APIs.
- Injected Registry, Ledger, approval, workspace, runtime, verification,
  review, integration, clock, and ID ports.

## Forbidden Dependencies

- Direct OpenClaw SDK/raw IPC parser, direct provider/model clients, hidden
  filesystem/Git mutation, adapter-to-adapter calls, reviewer mutation ports,
  and direct Guardian trust-root access.

## Failure, Compatibility, And Migration

Intent failure blocks an action. Unknown commit/effect outcome produces
reconciliation. Port/schema changes require compatibility adapters and version
evidence. The V1 Node fake remains characterization only; V2 live runtime and
PostgreSQL behavior require later tickets.

## Acceptance Gates

| Gate | Evidence | Owner | Required for merge |
|---|---|---|---|
| Gateway routing | closed action and exact binding tests | Engineering | yes when implemented |
| Call order | intent/effect/outcome/stop/review tests | Engineering | yes when implemented |
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

