---
module_id: task-ledger
name: Task Ledger
version: 1.0
status: active
owner: LATTICE maintainers
last_reviewed: 2026-07-29
---

## Mission

Provide the single durable control-plane truth by atomically appending,
verifying, sanitizing, replaying, and projecting hash-chained task events.

## Non-Goals

- Decide policy or legal transitions.
- Execute Runtime, Git, Scope Check, review, or integration actions.
- Store credentials, tokens, environment dumps, or raw secrets.
- Provide distributed multi-host consensus in Phase 1.

## Owned Data

- Per-task append-only event streams.
- Event sequence, predecessor hash, event hash, and command receipts.
- Sanitized event payloads.

Only the Orchestrator may request workflow appends. Other modules may read
verified projections through public contracts and cannot write ledger files.

## Public Contracts

- Append an event with `expected_sequence` and `command_id`.
- Return the prior receipt for an identical idempotent command.
- Reject a reused command ID whose content differs.
- Read and cryptographically verify a task stream.
- Replay a stream through Task Domain projection logic.
- Verify an entire stream without mutating it.

## Invariants

1. Sequence begins at one and increases by exactly one.
2. Every event hash covers the sanitized event and predecessor hash.
3. Append refuses a corrupt existing chain.
4. A sequence conflict never appends data.
5. Secret-bearing keys and values never reach durable payload fields.
6. Task status is derived from events, not separately persisted.

## Allowed Dependencies

- Task Domain public projection contract.
- Node.js filesystem and cryptographic standard-library APIs.
- Injected clock/ID sources for deterministic tests.

## Forbidden Dependencies

- Policy Engine, Git/Workspace, Scope Check, OpenClaw, real Runtime, network
  services, or direct model clients.

## Failure, Compatibility, And Migration

Intent-append failure blocks the side effect. Outcome-append failure after a
side effect requires a `BLOCKED` reconciliation path and must not silently
advance. Event schema changes require versioned readers and migration tests.
Automatic deletion or repair of a corrupt stream is forbidden.

## Acceptance Gates

| Gate | Evidence | Owner | Required for merge |
|---|---|---|---|
| Hash/replay tests | `node --test test/task-ledger.test.js` | Engineering | yes |
| Idempotency/sequence tests | duplicate and conflict cases | Engineering | yes |
| Tamper/redaction tests | corrupt/truncated/secret fixtures | Security review | yes |
| Full verification | `npm run verify` | Engineering | yes |

## Change Policy

Event identity, hashing, sanitization, write authority, replay, or failure
semantics require a versioned amendment, ADR, compatibility plan, and
responsible-human approval.

## Amendment History

| Version | Date | Decision reference | Summary | Approver |
|---|---|---|---|---|
| 1.0 | 2026-07-29 | ADR-001 | Initial single-truth ledger | Current user task |

