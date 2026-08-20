---
module_id: codex-adapter
name: Codex App Server Adapter
version: 1.2
status: active
owner: LATTICE maintainers
last_reviewed: 2026-08-09
---

## Mission

Own one supervised Codex app-server child process and translate the typed
`DeliveryCodexPort` contract to its version-pinned stdio protocol for the
bounded delivery composition.

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

- Implement `DeliveryCodexPort::run_delivery` and
  `DeliveryCodexPort::interrupt_delivery` with request-bound typed evidence.
- Do not implement or activate the frozen generic `CodexPort` as an alternate
  production writer path.
- Spawn the configured Codex binary with `app-server --listen stdio://`,
  initialize it, and bind every run to one exact working directory and request.
- Accept a controlled task run only when its request includes the exact Task
  Spec digest and Orchestrator-verified live Writer Lease identity, fencing
  token, current-head commitment, holder/worktree claim, and durable intent.
- Bind the child/thread/turn evidence to that same spec/lease/fence/worktree
  identity; never accept a caller-supplied lease, fence, thread, prompt, path,
  or permission mode.
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
5. Fake/synthetic, expired, suspect, stale, receipt-only, cross-spec,
   cross-worktree, or cross-fence authority cannot start a production turn.
6. Loss of lease-currentness or heartbeat requires bounded interruption and a
   reconciliation result unless terminal non-mutation is proved.
7. The adapter owns no lease repository and cannot acquire, renew, release, or
   project current authority; it consumes only the exact typed writer request
   ordered by Orchestrator.

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

Version 1.2 preserves the same sole `DeliveryCodexPort` process/protocol lane
and adds mandatory Task-Spec/Writer-Lease/fence/worktree binding. It adds no
lease repository, PostgreSQL dependency, caller-selected prompt/path, generic
Codex port activation, or second writable child.

## Acceptance Gates

| Gate | Evidence | Owner | Required for merge |
|---|---|---|---|
| Binary identity | exact path/version/digest and same-binary schema digest | Engineering | yes |
| Protocol lifecycle | initialize, thread, turn, terminal, interrupt tests | Engineering | yes |
| Real bounded run | app-server modifies only an isolated acceptance repo | Engineering | yes |
| Failure closure | EOF/timeout/malformed/ambiguous cases never report success | Engineering | yes |
| Writer authority | fake/synthetic/stale/cross-spec/cross-fence/current-head substitution matrix blocks spawn | Security review | yes |
| Lease loss | currentness-loss interruption/reconciliation and zero-later-success evidence | Engineering | yes |

## Change Policy

Writable-thread ownership, executable trust, protocol methods, permission
mapping, cancellation, or success semantics require a versioned constitution
amendment and architecture review.

## Amendment History

| Version | Date | Decision reference | Summary | Approver |
|---|---|---|---|---|
| 1.0 | 2026-08-05 | SPEC-002 v24, TASK-032 | First supervised Codex app-server boundary | Current user delivery-first directive |
| 1.1 | 2026-08-05 | SPEC-002 v26, ADR-021 clarification, TASK-032 | Bind the production adapter explicitly to the approved typed `DeliveryCodexPort`; generic writer port remains frozen | User approval of typed delivery contracts/ports in preceding implementation window |
| 1.2 | 2026-08-09 | SPEC-003 v3, ADR-023, TASK-038 | Bind every controlled Codex turn to one Task Spec, live Writer Lease current head, fencing token, and worktree claim | User TASK-038-first direction |
