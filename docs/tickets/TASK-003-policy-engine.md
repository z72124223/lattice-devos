---
ticket_id: TASK-003
spec_id: SPEC-001
module_id: policy-engine
constitution_version: 1.0
status: blocked
parallel_safe: false
depends_on:
  - TASK-001
allowed_paths:
  - src/policy/**
  - src/index.js
  - test/policy-engine.test.js
  - PLANS.md
  - docs/tickets/TASK-003-policy-engine.md
  - docs/workflow/WORKFLOW_LEDGER.md
likely_files:
  - src/policy/roles.js
  - src/policy/policy-engine.js
  - src/policy/approval.js
  - test/policy-engine.test.js
branch: feature/phase1-controlled-swarm
---

## Objective

Deliver the fail-closed role/action/capability matrix and exact-subject
execution/merge approval validation for the Phase 1 envelope.

## Acceptance Criteria

- [ ] SPEC-001 AC-03.
- [ ] SPEC-001 AC-04.
- [ ] Unknown inputs and every protected Phase 1 action deny with stable
  reasons.
- [ ] More than four active workers and any non-Implementer code write deny.

## Non-Goals

- Persist approvals, authenticate a live channel, run actions, or obtain locks.

## Module And Constitution Constraints

Use `policy-engine` v1.0. The module is pure except for the injected approval
verifier; it cannot mutate state.

## Dependencies And Overlap

Blocked on TASK-001 enums/spec. Not parallel-safe because the Orchestrator and
Workspace public contracts depend on its reason/permission model.

## TDD Behaviors

1. Unknown role/action/state/capability defaults to deny.
2. Role matrix allows only documented capabilities.
3. Phase 1 protected actions always deny.
4. Execution approval accepts only current spec hash/revision/owner evidence.
5. Merge approval accepts only frozen reviewed commit/diff subject.
6. Expired, replayed, wrong-kind/task/revision/subject approval denies.
7. Agent/budget limits deny unsafe envelopes.

## Verification

| Check | Command or service | Expected evidence |
|---|---|---|
| Focused policy tests | `node --test test/policy-engine.test.js` | exit 0 |
| Full current suite | `npm test` | exit 0 |

## Human Gate

Live owner/channel authentication is deferred to Phase 3; no Phase 1 blocker.

