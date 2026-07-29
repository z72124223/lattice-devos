---
ticket_id: TASK-003
spec_id: SPEC-001
module_id: policy-engine
constitution_version: 1.0
status: complete
parallel_safe: false
depends_on:
  - TASK-001
allowed_paths:
  - src/policy/**
  - src/index.js
  - test/policy-engine.test.js
  - PLANS.md
  - docs/tickets/TASK-003-policy-engine.md
  - docs/tickets/TASK-004-workspace-git.md
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

- [x] SPEC-001 AC-03.
- [x] SPEC-001 AC-04.
- [x] Unknown inputs and every protected Phase 1 action deny with stable
  reasons.
- [x] More than four active workers and any non-Implementer code write deny.

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

## TDD Evidence

| Behavior | RED evidence | GREEN evidence |
|---|---|---|
| Role/action/default-deny matrix | focused test exit 1, missing Policy Engine module | focused test exit 0 |
| Protected actions | passed on first focused run using the initial fail-closed ordering | complete all-role table passed; no fabricated RED |
| Execution approval | focused test exit 1, missing `verifyExecutionApproval` | focused test exit 0 |
| Merge approval | focused test exit 1, missing `verifyMergeApproval` | focused test exit 0 |
| Worker limit | focused test exit 1, missing `admitWorkers` | focused test exit 0 |
| Full matrix and stale lease | passed as regression coverage of documented matrix/fencing checks | focused test exit 0 |

Final ticket evidence:

- `node --test test/policy-engine.test.js`: 7 passed, exit 0.
- Exact role/non-protected-action matrix traversed.
- Every protected action tested against every role.
