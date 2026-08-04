---
module_id: codex-adapter
name: Codex App Server Adapter
version: 1.0
status: active
owner: LATTICE maintainers
last_reviewed: 2026-08-05
---

## Mission

Own one supervised Codex app-server child process and translate the typed
`CodexPort` contract to its version-pinned stdio protocol.

## Non-Goals

- Decide task policy, workflow order, approval, scope, test, Git, or release
  outcomes.
- Access PostgreSQL, OpenClaw, Graphify, Hermes, product credentials, or a
  second writable Codex thread.
- Report success before the app-server emits an unambiguous terminal result.

## Owned Data

- Executable path, version, digest, generated protocol-schema digest, process
  identity, native thread/turn identity, normalized events, and completion or
  reconciliation evidence.
- No task truth, product files, Git authority, approval, or durable database
  state.

## Public Contracts

- Implement `CodexPort::run` and `CodexPort::interrupt` with typed evidence.
- Spawn the configured Codex binary with `app-server --listen stdio://`,
  initialize it, and bind every run to one exact working directory and request.
- Normalize protocol events without treating unknown notifications as task
  authority.

## Invariants

1. One adapter instance owns at most one writable app-server child and native
   task thread at a time.
2. Executable version, file digest, and same-binary generated schema digest are
   captured before a run is accepted.
3. EOF, timeout, malformed protocol, or ambiguous completion fails closed as
   reconciliation-required evidence.
4. The adapter never calls Git, tests, PostgreSQL, or another component adapter.

## Allowed Dependencies

- `lattice-contracts` and `lattice-ports` public APIs.
- Rust process, async I/O, JSON, hashing, timeout, and path libraries needed for
  the versioned stdio client.
- The configured official `codex` executable.

## Forbidden Dependencies

- Orchestrator internals, direct model/provider APIs, database clients, Git
  mutation libraries, OpenClaw SDK, Graphify, Hermes, Guardian, or product
  credential stores.

## Failure, Compatibility, And Migration

Unknown executable identity, unsupported protocol/schema, initialization
failure, process loss, cancellation uncertainty, or terminal ambiguity blocks
success and returns typed failure or reconciliation evidence. Protocol changes
require a compatibility update; no in-place silent fallback is allowed.

## Acceptance Gates

| Gate | Evidence | Owner | Required for merge |
|---|---|---|---|
| Binary identity | exact path/version/digest and same-binary schema digest | Engineering | yes |
| Protocol lifecycle | initialize, thread, turn, terminal, interrupt tests | Engineering | yes |
| Real bounded run | app-server modifies only an isolated acceptance repo | Engineering | yes |
| Failure closure | EOF/timeout/malformed/ambiguous cases never report success | Engineering | yes |

## Change Policy

Writable-thread ownership, executable trust, protocol methods, permission
mapping, cancellation, or success semantics require a versioned constitution
amendment and architecture review.

## Amendment History

| Version | Date | Decision reference | Summary | Approver |
|---|---|---|---|---|
| 1.0 | 2026-08-05 | SPEC-002 v24, TASK-032 | First supervised Codex app-server boundary | Current user delivery-first directive |
