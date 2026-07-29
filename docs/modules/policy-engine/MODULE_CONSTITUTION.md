---
module_id: policy-engine
name: Policy Engine
version: 1.0
status: active
owner: LATTICE maintainers
last_reviewed: 2026-07-29
---

## Mission

Return deterministic, fail-closed authorization decisions for roles, actions,
states, approvals, capabilities, budgets, and writer leases.

## Non-Goals

- Change task state or append ledger events.
- Execute an authorized action.
- Authenticate a real Telegram/OpenClaw/account identity in Phase 1.
- Repair scope violations or break locks.

## Owned Data

- Role-to-capability matrix.
- Protected action and Phase 1 deny sets.
- Approval-subject validation rules.
- Maximum worker-agent and offline budget rules.
- Stable policy reason codes.

The Policy Engine owns decisions, not Task Specs, approval persistence, leases,
or side effects.

## Public Contracts

- Evaluate `role × action × state × approval × lease × task spec`.
- Verify exact execution and merge approval subjects through an injected owner
  verifier.
- Validate the Phase 1 budget/capability envelope.
- Return `{allowed, reason_code, evidence}` without side effects.

## Invariants

1. Unknown roles, actions, states, capabilities, and authority default to deny.
2. Only Implementer can receive product-code write permission.
3. Integrator can mutate Git metadata but cannot edit product files.
4. Missing, stale, expired, replayed, or mismatched approvals deny.
5. Phase 1 real model, network, deployment, payment, credentials, publication,
   permanent deletion, and playmate access always deny.
6. `max_agents` above four always denies.

## Allowed Dependencies

- Task Domain public enums and immutable Task Spec types.
- Injected approval verifier.

## Forbidden Dependencies

- Filesystem, Git, Task Ledger writes, Scope Check, Runtime, OpenClaw runtime,
  account services, or network.

## Failure, Compatibility, And Migration

Policy evaluation errors deny with a stable internal-error reason. Expanding a
permission, action, role, capability, or risk class is a security-sensitive
change and must not be backfilled automatically.

## Acceptance Gates

| Gate | Evidence | Owner | Required for merge |
|---|---|---|---|
| Matrix tests | `node --test test/policy-engine.test.js` | Engineering | yes |
| Unknown-default-deny tests | table-driven unknown cases | Security review | yes |
| Approval substitution/replay | exact-subject regression tests | Security review | yes |
| Full verification | `npm run verify` | Engineering | yes |

## Change Policy

Any permission expansion or approval/risk/budget change requires a versioned
amendment, security and architecture review, and explicit responsible-human
approval.

## Amendment History

| Version | Date | Decision reference | Summary | Approver |
|---|---|---|---|---|
| 1.0 | 2026-07-29 | SPEC-001, ADR-002 | Initial fail-closed policy | Current user task |

