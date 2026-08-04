---
ticket_id: TASK-005
spec_id: SPEC-001
module_id: scope-check
constitution_version: 1.0
status: superseded
superseded_by: SPEC-002
parallel_safe: false
depends_on:
  - TASK-001
  - TASK-004
allowed_paths:
  - src/scope/**
  - src/index.js
  - test/scope-check.test.js
  - PLANS.md
  - docs/tickets/TASK-005-scope-check.md
  - docs/workflow/WORKFLOW_LEDGER.md
likely_files:
  - src/scope/scope-check.js
  - test/scope-check.test.js
branch: feature/phase1-controlled-swarm
---

> Do not execute. The V2 direction replaces the Node.js ticket set; retained as
> historical characterization scope only.

## Objective

Deliver a read-only deterministic scope report over normalized Git change
records, including rename/operation/path/link escape cases.

## Acceptance Criteria

- [ ] SPEC-001 AC-06.
- [ ] Identical rules/evidence produce stable rule/evidence/report hashes.
- [ ] Scope report explicitly labels itself detection-only.

## Non-Goals

- Sandbox, prevention, repair, revert, cleanup, staging, or task transition.

## Module And Constitution Constraints

Use `scope-check` v1.0. Forbidden overrides allowed; unknown operations and link
kinds deny; no filesystem mutation.

## Dependencies And Overlap

Blocked on Task Scope contract and normalized Git change evidence. Not
parallel-safe because Orchestrator verification depends on the report format.

## TDD Behaviors

1. Accept exact/prefix/glob allowed paths and operations.
2. Deny forbidden override and out-of-scope path.
3. Deny absolute, empty, traversal, `.git`, escaped, symlink, and junction.
4. Check both paths of rename.
5. Deny unknown operation/kind.
6. Stable-sort violations and hash rules/evidence/report.
7. Prove input records remain unchanged.

## Verification

| Check | Command or service | Expected evidence |
|---|---|---|
| Focused scope tests | `node --test test/scope-check.test.js` | exit 0 |
| Full current suite | `npm test` | exit 0 |

## Human Gate

Real Runtime sandbox/containment remains Phase 3; no Phase 1 blocker.
