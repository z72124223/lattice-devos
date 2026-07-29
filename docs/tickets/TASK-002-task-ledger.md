---
ticket_id: TASK-002
spec_id: SPEC-001
module_id: task-ledger
constitution_version: 1.0
status: complete
parallel_safe: false
depends_on:
  - TASK-001
allowed_paths:
  - src/ledger/**
  - src/index.js
  - test/task-ledger.test.js
  - PLANS.md
  - docs/tickets/TASK-002-task-ledger.md
  - docs/tickets/TASK-003-policy-engine.md
  - docs/workflow/WORKFLOW_LEDGER.md
likely_files:
  - src/ledger/task-ledger.js
  - src/ledger/sanitize.js
  - test/task-ledger.test.js
branch: feature/phase1-controlled-swarm
---

## Objective

Deliver a per-task append-only ledger that is the only durable workflow truth,
supports deterministic replay/idempotency, and detects corruption without
storing secrets.

## Acceptance Criteria

- [x] SPEC-001 AC-07.
- [x] Sequence conflict and changed duplicate command fail closed.
- [x] Restart replay produces the same Task Packet projection.

## Non-Goals

- Policy decisions, Git, Runtime, or distributed consensus.

## Module And Constitution Constraints

Use `task-ledger` v1.0 and Task Domain public projection only. No external
module may write ledger files.

## Dependencies And Overlap

Blocked on TASK-001 public state/spec contract. Not parallel-safe because it
updates shared public exports and becomes Orchestrator state authority.

## TDD Behaviors

1. Append and verify the first event.
2. Enforce expected sequence and predecessor hash.
3. Return prior receipt for identical `command_id`.
4. Reject changed-content command reuse.
5. Detect changed/reordered/truncated content.
6. Sanitize nested secret-bearing fields.
7. Replay the same state after a new ledger instance opens the stream.

## Verification

| Check | Command or service | Expected evidence |
|---|---|---|
| Focused ledger tests | `node --test test/task-ledger.test.js` | exit 0 |
| Full current suite | `npm test` | exit 0 |

## Human Gate

none

## TDD Evidence

| Behavior | RED evidence | GREEN evidence |
|---|---|---|
| First durable hash-chained event | focused test exit 1, missing `task-ledger.js` | focused test exit 0 |
| Sequence/idempotency | passed on first focused run because the first safe append already serialized and fingerprinted commands | focused test exit 0; no fabricated RED |
| Restart Task Packet replay | focused test exit 1, missing `readTaskPacket` | focused test exit 0 |
| Tamper/tail detection | passed on first focused run as regression coverage of event/head verification | focused test exit 0; no fabricated RED |
| Secret redaction | passed on first focused run as regression coverage of the append sanitizer | focused test exit 0; no fabricated RED |

Final ticket evidence:

- `node --test test/task-ledger.test.js`: 5 passed, exit 0.
- `npm test`: 11 passed, exit 0.
- `npm run check`: `check=ok files=46 constitutions=7`, exit 0.
- `git diff --check`: exit 0.
