---
ticket_id: TASK-006
spec_id: SPEC-001
module_id: orchestrator-runtime
constitution_version: 1.0
status: blocked
parallel_safe: false
depends_on:
  - TASK-001
  - TASK-002
  - TASK-003
  - TASK-004
  - TASK-005
allowed_paths:
  - src/orchestrator/**
  - src/runtime/**
  - src/index.js
  - test/orchestrator.test.js
  - test/controlled-swarm.e2e.test.js
  - PLANS.md
  - docs/tickets/TASK-006-orchestrator-fake-runtime.md
  - docs/workflow/WORKFLOW_LEDGER.md
likely_files:
  - src/orchestrator/lattice-orchestrator.js
  - src/runtime/fake-runtime.js
  - test/orchestrator.test.js
  - test/controlled-swarm.e2e.test.js
branch: feature/phase1-controlled-swarm
---

## Objective

Deliver the sole application command entry and deterministic Fake Runtime,
proving the entire plan/approval/writer/verify/review/merge flow and every
fail-closed stop point without external calls.

## Acceptance Criteria

- [ ] SPEC-001 AC-09.
- [ ] SPEC-001 AC-10.
- [ ] Intent ledger failure blocks side effects.
- [ ] Runtime stop and writer revocation occur before review.
- [ ] No more than four worker-agent invocations are active.

## Non-Goals

- Real Codex/OpenClaw process, network, model, account, or deployment.

## Module And Constitution Constraints

Use `orchestrator-runtime` v1.0. Orchestrator is the only event appender; Runtime
and Reviewers cannot receive a ledger mutation contract.

## Dependencies And Overlap

Blocked on all core public contracts. Not parallel-safe because it integrates
every module and fixes call ordering.

## TDD Behaviors

1. Submit safe plan and stop awaiting execution approval.
2. Deny stale/wrong approval before any workspace action.
3. Approved execution records intent, prepares worktree, and acquires writer.
4. Runtime success revokes writer before checks/reviews.
5. Every runtime/check/scope/reviewer failure stops at the exact gate.
6. Stop moves through `STOPPING`, interrupts runtime, revokes lease, and
   completes `CANCELLED`.
7. Review success freezes commit/diff and waits for merge approval.
8. Wrong/stale merge approval denies; current approval integrates.
9. Integration conflict blocks; clean fake integration reaches `COMPLETED`.
10. Duplicate command returns prior receipt without rerunning a side effect.

## Verification

| Check | Command or service | Expected evidence |
|---|---|---|
| Focused orchestration | `node --test test/orchestrator.test.js` | exit 0 |
| Full fake vertical slice | `node --test test/controlled-swarm.e2e.test.js` | exit 0 |
| Full current suite | `npm test` | exit 0 |

## Human Gate

none for Fake Runtime; Real Runtime is a later approved specification.

