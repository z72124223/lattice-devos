---
spec_id: SPEC-008
status: ready
version: 2
modules:
  - module_id: latticed
    constitution_version: 2.6
  - module_id: task-ledger
    constitution_version: 2.5
---

# Historical terminal status across runtime upgrades

## Problem

A verified historical task can become unreadable after the canonical
`latticed` binary changes because normal lifecycle replay also proves current
ingress write authority. The current production record is terminal `FAILED`;
reading that fact must not require or create successor write authority.

## Intended behavior

`lattice_delivery_status` and `lattice_task_status` may project a historical
non-success terminal only through a separate read-only replay path. That path
verifies the stored Task-created commitment and audit, the complete PostgreSQL
Task Ledger stream, autonomy receipt, legal state transitions, and the fixed
historical ingress identity family. It performs only the bounded database and
loopback reads needed for verification: no append, external mutation, model
execution, delivery effect, Graphify run, or Hermes run is permitted.

Normal lifecycle load, Submit, resume, transition, result recording, and every
other mutation continue to require the current ingress profile commitment.

## Goals

- Restore exact read-only visibility of verified historical `FAILED`,
  `REJECTED`, `BLOCKED`, or `CANCELLED` state after binary commitment drift.
- Preserve the existing MCP tool names, schemas, output fields, and task-ref
  comparison.
- Keep PostgreSQL schema v5, Memory v3, and Writer Lease v2 byte-identical.

## Non-goals

- Grant successor write, retry, resume, handoff, or effect authority.
- Project a historical non-terminal or `COMPLETED` task under a different
  current ingress commitment.
- Accept a legacy Task-Spec digest as a task-reference alias.
- Add an event kind, database object, migration, MCP field, or second truth.

## Security and compatibility

- Historical status validates exactly one closed audit shape; its client kind,
  actor kind, adapter ID, and event actor must match the current closed ingress
  family, while binary/schema/profile commitment bytes may differ.
- The historical commitment must be a nonzero SHA-256 value, must reproduce the
  stored Task-created subject, and must reproduce the admission-observation
  commitment with the stored nonzero process-start digest.
- Any malformed, substituted, cross-family, non-terminal, or completed stream
  fails closed.
- `lattice_task_status` still recomputes and compares the current-format task
  reference. Caller-supplied hexadecimal text never becomes bearer authority.
- The unpersistable source-only ingress-handoff path from ADR-024 is withdrawn
  under ADR-025 before deploy.

## Acceptance criteria

- [ ] Current-profile replay remains unchanged and strict.
- [ ] A successor binary can read a verified historical `FAILED` status without
      adding an event, command, outbox item, or database row.
- [ ] The same successor remains unable to Submit, resume, transition, or record
      a result for that historical stream.
- [ ] Historical non-terminal and `COMPLETED` streams remain rejected.
- [ ] Audit, commitment, subject, actor, observation, autonomy, transition, or
      task-reference substitution remains rejected.
- [ ] Public status contains no profile, binary, process, audit, or secret data.
- [ ] Fresh-process PostgreSQL acceptance proves the current production record
      is readable and no durable bytes changed.

## Verification plan

Focused Rust tests cover strict-versus-historical replay, terminal filtering,
tamper cases, task-reference rejection, and zero mutation. A disposable
PostgreSQL run proves schema-v5 compatibility before a backed-up local
deployment and fresh MCP status check.

## Human decisions

The user authorized the bounded repair, push, product-branch merge, and local
deployment in this task. A future writable A-to-B handoff remains a separate
schema/architecture decision.

## Open questions

None for this read-only repair.
