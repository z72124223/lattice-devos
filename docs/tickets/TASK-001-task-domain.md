---
ticket_id: TASK-001
spec_id: SPEC-001
module_id: task-domain
constitution_version: 1.0
status: ready
parallel_safe: false
depends_on: []
allowed_paths:
  - .gitattributes
  - .gitignore
  - .github/workflows/ci.yml
  - package.json
  - package-lock.json
  - README.md
  - schemas/task-packet.schema.json
  - scripts/check-project.mjs
  - src/domain/**
  - src/index.js
  - test/task-domain.test.js
  - PLANS.md
  - docs/tickets/TASK-001-task-domain.md
  - docs/workflow/WORKFLOW_LEDGER.md
likely_files:
  - src/domain/task-spec.js
  - src/domain/task-state.js
  - schemas/task-packet.schema.json
  - test/task-domain.test.js
branch: feature/phase1-controlled-swarm
---

## Objective

Create the dependency-free repository/test foundation and deliver a public Task
Domain contract that validates and freezes a safe Phase 1 Task Spec, detects DAG
cycles, hashes immutable approval subjects, and enforces the transition graph.

## Acceptance Criteria

- [ ] SPEC-001 AC-01.
- [ ] SPEC-001 AC-02.
- [ ] Same normalized spec yields the same hash; any approval-relevant change
  changes it.
- [ ] Unsafe Phase 1 budgets, runtime/network/deployment settings, paths, and
  dependency cycles fail with stable reason codes.

## Non-Goals

- Persistence, role authorization, locks, Git, Runtime, or OpenClaw.

## Module And Constitution Constraints

Use `task-domain` v1.0. Keep the implementation pure and I/O-free; mutable
status stays outside `spec_hash`.

## Dependencies And Overlap

No dependency. Not parallel-safe because it establishes schema, exports, root
test/check scripts, and contracts used by every later ticket.

## TDD Behaviors

1. Reject invalid schema/version/Phase 1 envelopes.
2. Accept and normalize the minimal safe Task Spec.
3. Produce deterministic hash and changed-field invalidation.
4. Detect dependency cycles.
5. Allow every main-path/exception transition and reject every invalid edge.
6. Project an initial packet as `AWAITING_EXECUTION_APPROVAL`.

## Verification

| Check | Command or service | Expected evidence |
|---|---|---|
| Focused domain tests | `node --test test/task-domain.test.js` | exit 0 |
| Project checks | `npm run check` | syntax/JSON/docs exit 0 |

## Human Gate

none

