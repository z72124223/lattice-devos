---
module_id: task-domain
name: Task Domain
version: 1.0
status: active
owner: LATTICE maintainers
last_reviewed: 2026-07-29
---

## Mission

Define and validate the immutable Task Spec, deterministic specification hash,
task states, and legal transition graph so every adapter uses the same workflow
language.

## Non-Goals

- Persist events or current status.
- Decide actor authorization.
- Run Git, processes, models, network calls, or OpenClaw.
- Maintain a second mutable task record.

## Owned Data

- Task Spec JSON Schema version `1.0`.
- Task state and transition enumerations.
- Canonical Task Spec hashing rules.
- Pure projection shape returned from replayed events.

The Task Ledger owns persisted instances and events. This module mutates no
external data.

## Public Contracts

- Validate and normalize a proposed Phase 1 Task Spec.
- Derive `spec_hash` from canonical immutable fields.
- Validate a requested transition and return a stable allow/deny reason.
- Project a Task Packet view from immutable spec plus replayed evidence.
- Detect cyclic task dependencies supplied as a DAG.

## Invariants

1. Mutable status is never part of `spec_hash`.
2. Unknown schema versions, states, and transitions fail closed.
3. The same normalized Task Spec always produces the same hash.
4. A changed approval-relevant field changes the hash.
5. Task-domain functions perform no I/O and have no hidden clock.

## Allowed Dependencies

- Node.js standard-library primitives for hashing and URL/path-independent data.
- No other LATTICE module is required for core validation.

## Forbidden Dependencies

- Filesystem, subprocess, Git, network, OpenClaw, Task Ledger adapters, Policy
  Engine, Orchestrator, or Runtime adapters.

## Failure, Compatibility, And Migration

Validation returns stable error codes and never partially accepts a spec.
Unknown schema versions require an explicit migration design and ADR. Version
`1.0` data is not silently rewritten.

## Acceptance Gates

| Gate | Evidence | Owner | Required for merge |
|---|---|---|---|
| Schema examples | `node --test test/task-domain.test.js` | Engineering | yes |
| Transition table | all allowed/forbidden edges tested | Engineering | yes |
| Hash stability | canonicalization regression tests | Engineering | yes |
| Full verification | `npm run verify` | Engineering | yes |

## Change Policy

Mission, schema fields, hash subject, states, public contracts, or transition
rules require a versioned amendment, specification update, architecture review,
and explicit responsible-human approval.

## Amendment History

| Version | Date | Decision reference | Summary | Approver |
|---|---|---|---|---|
| 1.0 | 2026-07-29 | SPEC-001, ADR-001 | Initial offline task contract | Current user task |

