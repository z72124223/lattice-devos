---
module_id: hermes-adapter
name: Hermes Adapter
version: 1.0
status: active
owner: LATTICE maintainers
last_reviewed: 2026-08-20
---

## Mission

Own one supervised Hermes research/reflection lane and translate the typed
Hermes port contract into a version-pinned, schema-valid candidate envelope for
the bounded delivery composition.

## Non-Goals

- Decide task policy, workflow order, approval, scope, test, Git, or release
  outcomes.
- Access PostgreSQL, OpenClaw, Graphify, product credentials, or a writable
  Codex thread.
- Report LATTICE authority from Hermes internal memory, guard settings, or
  model output alone.

## Owned Data

- Hermes executable/profile identity, version, digest, requested capability
  set, normalized reflection events, candidate envelopes, and completion or
  reconciliation evidence.
- No task truth, product files, Git authority, approval, or durable database
  state.

## Public Contracts

- Implement the typed Hermes port boundary with request-bound evidence.
- Do not implement a second writable Codex path or any general product-writer
  lease.
- Run Hermes with a dedicated profile, read-only product input, and separate
  candidate output directory.
- Preserve required provenance, schema validation, and quarantine behavior for
  rejected output.

## Invariants

1. One adapter instance owns at most one Hermes run at a time.
2. Executable version, file digest, and exact profile/capability evidence are
   captured before a run is accepted.
3. EOF, timeout, malformed output, or ambiguous completion fails closed as
   reconciliation-required evidence.
4. The adapter never calls Git, tests, PostgreSQL, or another component
   adapter.

## Allowed Dependencies

- `lattice-contracts` and `lattice-ports` public APIs.
- Rust process, async I/O, JSON, hashing, timeout, and path libraries needed
  for the versioned Hermes client.
- The configured official Hermes executable.

## Forbidden Dependencies

- Orchestrator internals, direct model/provider APIs, database clients, Git
  mutation libraries, OpenClaw SDK, Graphify, Codex writer ownership, or
  product credential stores.

## Failure, Compatibility, And Migration

Unknown executable identity, unsupported capability set, initialization
failure, process loss, cancellation uncertainty, or terminal ambiguity blocks
success and returns typed failure or reconciliation evidence. Protocol changes
require a compatibility update; no in-place silent fallback is allowed.

## Acceptance Gates

| Gate | Evidence | Owner | Required for merge |
|---|---|---|---|
| Binary identity | exact path/version/digest and profile evidence | Engineering | yes |
| Protocol lifecycle | initialize, run, terminal, interrupt tests | Engineering | yes |
| Real bounded run | Hermes returns only a schema-valid candidate envelope | Engineering | yes |
| Failure closure | EOF/timeout/malformed/ambiguous cases never report success | Engineering | yes |

## Change Policy

Writable Hermes capability, executable trust, protocol methods, or success
semantics require a versioned constitution amendment and architecture review.

## Amendment History

| Version | Date | Decision reference | Summary | Approver |
|---|---|---|---|---|
| 1.0 | 2026-08-20 | SPEC-002 v27, ADR-006, TASK-039 | First supervised Hermes adapter boundary for the broker protocol | Current user delivery-first directive |
