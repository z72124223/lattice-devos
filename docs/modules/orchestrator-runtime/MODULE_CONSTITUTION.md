---
module_id: orchestrator-runtime
name: Orchestrator and Runtime Port
version: 1.0
status: active
owner: LATTICE maintainers
last_reviewed: 2026-07-29
---

## Mission

Provide the single application entry that orders policy, ledger, workspace,
runtime, verification, review, and integration calls, plus a deterministic Fake
Runtime implementing the future real-runtime port.

## Non-Goals

- Maintain a second task state store.
- Bypass Policy, approval, scope, lease, review, or ledger gates.
- Call a real model, network, Hostinger, OpenClaw, or user project in Phase 1.
- Let Runtime or Reviewers write ledger events directly.

## Owned Data

- Command orchestration and call ordering.
- Runtime/Reviewer/Verifier port contracts.
- Fake Runtime scripted scenarios, call receipts, virtual time, and cancellation
  observations.

Task Domain owns states/spec; Task Ledger owns durable truth; Policy owns
authorization; Workspace owns leases/Git; Scope owns reports.

## Public Contracts

- Submit plan, approve execution, execute, stop, approve merge, and integrate.
- Require a unique `command_id` and `correlation_id`.
- Record intent before every side effect and outcome after it.
- Revoke writer authority before verification/review.
- Stop on the first failed gate with a stable task outcome.
- Expose a deterministic Fake Runtime with success/failure/timeout/cancel/
  malformed/scope/writer/reviewer/conflict scenarios.

## Invariants

1. Orchestrator is the only workflow event appender.
2. No side effect starts before intent evidence is durable.
3. Only one Implementer invocation runs with a current writer lease.
4. Runtime is stopped and lease revoked before read-only review.
5. Reviewers cannot access mutation ports.
6. Merge never starts without a separate exact-subject approval.
7. Phase 1 Fake Runtime has zero model, network, credential, and external cost.
8. Active worker-agent count never exceeds four.

## Allowed Dependencies

- Public contracts of Task Domain, Task Ledger, Policy Engine, Workspace/Git,
  and Scope Check.
- Injected clock, ID, approval, Runtime, Verification, Review, and Integration
  ports.

## Forbidden Dependencies

- Direct OpenClaw SDK dependency in core.
- Direct API/model/Hostinger/Telegram clients.
- Hidden filesystem or Git mutation outside injected ports.
- Runtime-to-Ledger or Reviewer-to-Ledger dependency.

## Failure, Compatibility, And Migration

Intent failure blocks the action. Outcome persistence failure after an external
action produces a blocked reconciliation result. Timeouts/cancel use virtual
time in Fake Runtime and an explicit interrupt contract in a future real
runtime. Port changes require compatibility adapters.

## Acceptance Gates

| Gate | Evidence | Owner | Required for merge |
|---|---|---|---|
| Call-order tests | `node --test test/orchestrator.test.js` | Engineering | yes |
| Fake scenario tests | all scripted outcomes | Engineering | yes |
| No-external-call proof | injected-only dependency assertions | Security review | yes |
| End-to-end flow | `node --test test/controlled-swarm.e2e.test.js` | Engineering | yes |

## Change Policy

Gate order, write authority, event ownership, port contracts, stop semantics, or
agent limit changes require a versioned amendment, spec/ADR update,
architecture review, and responsible-human approval.

## Amendment History

| Version | Date | Decision reference | Summary | Approver |
|---|---|---|---|---|
| 1.0 | 2026-07-29 | SPEC-001, ADR-001/002 | Initial deterministic control loop | Current user task |

