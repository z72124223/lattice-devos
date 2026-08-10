---
module_id: lattice-ports
name: LATTICE I/O Ports
version: 1.9
status: active
owner: LATTICE maintainers
last_reviewed: 2026-08-11
---

## Mission

Define the abstract Rust traits through which orchestration reaches the gateway,
sole product-code writer, read-only knowledge lane, untrusted research lane,
typed physical control store, durable delivery ledger, bounded workspace/Git
lane, fixed-test lane, authoritative Task lifecycle repository lane, and the
independent bounded Reflection journal lane.

Ports define provider-neutral capabilities. A concrete implementation may be an
in-process adapter, MCP provider, local RPC service, or HTTP provider only after
a reviewed adapter binds it to the same typed Port. The protocol never owns the
capability semantics or authority.

## Non-Goals

- Select or start OpenClaw, ChatGPT, Codex, Graphify, Hermes, PostgreSQL, or any
  provider process.
- Perform I/O, decide policy, own workflow order, or define domain transitions.
- Select a workspace, construct a command line, choose a test, create a Git
  commit, or interpret an MCP call.
- Manufacture PostgreSQL durability or external component compatibility; a
  concrete Store may return a structurally classified physical receipt whose
  durability still requires its own implementation evidence.
- Own a Reflection scheduler, global queue, `claim_next`, heartbeat, claim
  lease, timeout, orphan recovery, retention, batch classifier, or Hermes
  execution.
- Expose raw Task Ledger events, SQL, database clients, credentials, prompts,
  stderr, commands, paths, or provider-private payloads.

## Owned Data

- Port traits, external-port errors, and a component-free inbound
  `GatewayServiceError`.
- Bounded `TaskLifecycleError` and replay-derived `TaskLifecycleEvidence`
  transport values. Task Domain remains the semantic owner of `TaskState` and
  core transition legality.
- Bounded `TaskReflectionError`, `TaskReflectionEvidence`, typed history query,
  typed history page/event/reference values, and the three first-slice
  Reflection repository traits.
- No runtime, durable, product, credential, provider-session, queue-owner, or
  scheduler data.

## Public Contracts

- `GatewayService` accepts server-derived peer context plus one complete typed
  request and returns a typed bound reply; codec errors remain outside the
  service call.
- `GatewayService` returns `GatewayServiceResult`: Rust-core routing or
  reply-binding failures cannot be attributed to an external component.
- `DeliveryCodexPort` is the sole typed product-code mutation contract used by
  Orchestrator 2.3. Its request binds the complete Task Spec digest, durable
  intent, prepared workspace, and current Writer Lease/fencing evidence. The
  earlier generic `CodexPort` remains source-compatible only for pre-delivery
  consumers; it is frozen and cannot be wired as a second production writer
  beside the typed delivery lane.
- `GraphifyPort` returns derived read-only evidence.
- The generic `GraphifyPort` is frozen for pre-TASK-033 compatibility.
  Production graph-memory composition uses `CodeSnapshotPort` to materialize
  one exact tracked commit, `GraphifyAnalysisPort` to run and strictly parse
  the pinned code-only child, and `CodebaseMemoryPort` to persist/load exact
  analysis and bounded retrieval audit. None exposes a caller-selected path,
  command, environment, query, SQL, or credential.
- `HermesPort` returns untrusted candidate evidence.
- `DeliveryLedgerPort` records typed intent before an effect, records typed
  terminal outcome/receipt after it, and loads exact status without exposing a
  database client, SQL, credential, or schema detail.
- `TaskLifecyclePort` admits one exact Task binding/client request, appends one
  caller-validated core state transition, records one result digest, and loads
  one replay-derived authoritative lifecycle projection. It exposes no SQL,
  database client, event fragment, arbitrary payload, cache, or alternate state
  mutation, and it does not decide transition legality.
- `TaskReflectionQueuePort` is a known-Task first-slice boundary. It can
  idempotently ensure one completed core Task is pending, claim its exact
  generation, record one closed failure kind, mark a bounded retry, mark
  degraded evidence, and load the independent Reflection projection. It does
  not discover Tasks or own a global queue.
- `HermesTaskReflectionHistoryPort` returns an allowlisted typed, keyset-paged,
  bounded history whose digest commits its exclusive cursor, limit, returned
  events, next cursor, exact core anchor, and current journal head. It exposes
  only digests and closed event meanings, never raw evidence or candidate text.
- `HermesTaskReflectionCandidatePort` appends only one closed
  `ReflectionCandidateKind` plus candidate/history digests after replaying the
  exact authorized page. It cannot update/delete events, transition core Task
  state, claim writer authority, or promote a candidate to truth.
- `WorkspaceGitPort` prepares/inspects the preconfigured bounded workspace and
  creates a local commit only from typed passing scope/test evidence. It
  exposes no arbitrary command or caller-selected path.
- `TestRunnerPort` runs only the fixed test bound into the delivery request and
  returns typed test evidence; it accepts no shell text or command arguments.
- `ControlStore::transact` accepts one complete typed physical transaction and
  returns a typed terminal receipt or Store-specific error without defining
  domain legality.
- `ControlStore::current_head` takes mutable Store access and returns the
  independently retained physical head for one exact scope; this is not a
  domain-owner current head. The mutability is explicit because a synchronous
  driver query mutates connection state.
- Each trait returns its own evidence type so a provider cannot cross-label
  another component or authority boundary.

## Provider-Neutral Boundary

1. Define and version the Port before choosing MCP, HTTP, local RPC, or an
   in-process implementation.
2. No protocol-specific session, request, transport, authentication, process,
   or schema type enters this crate.
3. MCP implementations live at the edge and may not widen a Port signature or
   reinterpret a typed rejection as success.
4. Extracting an existing adapter into a provider process requires a complexity
   justification under ADR-024 and must show lower total coupling or stronger
   isolation without creating another truth, writer, scheduler, or queue owner.

## Invariants

1. This crate depends only on `lattice-contracts` and Task Domain 2.3. The Task
   Domain dependency is limited to closed Task/Reflection values used by typed
   signatures; no validation/planning implementation enters Ports.
2. OpenClaw and ChatGPT are inbound gateway clients, never second control cores
   or outbound providers selected by orchestration.
3. Traits expose no concrete database, filesystem, network, MCP, or process
   type.
4. A port cannot return another lane's evidence type.
5. Port errors are explicit and unknown outcomes never imply success.
6. No adapter calls another adapter through these traits.
7. `GatewayService` never returns gateway-client-produced evidence as a
   substitute for a Rust-core routing reply.
8. A `GatewayServiceError` has a stable kind/code but no `Component`; generic
   outbound adapter `PortError` values carry an external component, while the
   Store uses its complete Store-specific error type.
9. Store outcomes distinguish invalid/substituted, unauthorized/admission,
   capacity/overflow, unavailable/serialization, corruption, unknown outcome,
   and stale physical head without representing any as success. Stale head is
   a terminal denial receipt, not an error or applied mutation.
10. Store traits expose no SQL, table/schema/path, arbitrary row, driver,
    connection, migration, or domain-transition type.
11. A terminal physical receipt may classify its own fake or PostgreSQL
    durability, but the port never upgrades that evidence into domain legality,
    freshness, effect delivery, Guardian authority, or release authority.
12. Delivery, workspace/Git, test, and Reflection traits expose no concrete
    driver, process, filesystem handle, command line, SQL, credential, or MCP
    type.
13. Port methods do not own effect order. Only the injected orchestrator may
    sequence intent, Codex, workspace/test/Git, and terminal persistence.
14. Graph-memory ports expose no concrete Git, process, filesystem, JSON,
    database, driver, transaction, staging-directory, or Graphify CLI type.
15. Snapshot materialization, analysis, persistence, and retrieval are
    distinct effects. A failed or uncertain earlier port cannot be represented
    as a later success.
16. The workspace/Git port cannot commit before receiving request-bound passing
    scope and fixed-test evidence.
17. Unknown ledger, Codex, workspace, test, Git, or Reflection outcome never
    becomes a successful port result.
18. The Task lifecycle port owns no Task Domain legality or workflow order.
    Orchestrator supplies validated typed transitions/effects and the adapter
    delegates append/replay to Task Ledger semantics.
19. A Task-control status load performs no workspace, Codex, verification,
    Git, Graphify, Hermes, or Memory effect and returns no raw event,
    diagnostic, prompt, command, path, SQL, secret, lease/fence, or child
    output.
20. No task or Reflection port accepts caller-selected actor authority, project
    path, verification command, writer lease, fencing token, or Codex thread.
21. Writer Lease repository semantics/traits remain owned by Writer Lease 1.1;
    this crate neither duplicates nor wraps them into a second authority.
22. Core `TaskLifecycleEvidence::core_head_digest` remains anchored to the last
    legal core event while the whole append-only journal head may advance.
23. Reflection evidence never becomes `TaskLifecycleEvidence` and cannot
    downgrade core `COMPLETED`.
24. History pages are bounded by
    `MAX_TASK_REFLECTION_HISTORY_EVENTS`; zero, over-limit, invalid cursor, or
    stale-page candidate attempts fail closed.
25. The first-slice queue traits are exact known-Task operations. They do not
    imply cross-Task discovery, unattended worker completion, or Hermes
    production acceptance.
26. Runtime-owned command namespaces cannot be supplied by external callers or
    retained by another event kind.

## Allowed Dependencies

- `lattice-contracts`.
- `lattice-task-domain` 2.3 only for closed `TaskState`, `ReflectionState`,
  `ReflectionFailureKind`, and `ReflectionCandidateKind` representations.
- Rust standard library.

## Forbidden Dependencies

- Concrete adapters, Orchestrator, policy implementation, database drivers,
  MCP/network/process clients, model SDKs, credentials, provider sessions,
  schedulers, and product repositories.

## Failure, Compatibility, And Migration

Rejected calls and exhausted/unknown outcomes return typed errors.

Version 1.6 explicitly recorded the approved `DeliveryCodexPort`
specialization used by Orchestrator 2.1 and froze the earlier generic
`CodexPort` outside the production delivery composition. This is one logical
writer lane with two versioned interface shapes, never two runtime writers.

Version 1.8 added the neutral Task lifecycle repository boundary and
lease-bound Codex-delivery request shape for Orchestrator 2.3. It retained the
contracts-only core boundary except for an explicit one-way Task Domain 2.2
dependency on the closed `TaskState` value.

Version 1.9 adds the independent known-Task Reflection queue, bounded history,
and digest-only candidate Ports plus the Reflection evidence/error transport
values checkpointed by GH-9. It adds no concrete database, MCP implementation,
provider execution, global queue, scheduler, cross-Task discovery, retention,
or authority. Existing 1.8 Task lifecycle signatures remain compatible except
for the documented `core_head_digest` projection required to preserve core
completion under append-only Reflection tails.

## Acceptance Gates

| Gate | Evidence | Owner | Required for merge |
|---|---|---|---|
| Port contract tests | `cargo test -p lattice-ports` | Engineering | yes |
| Store error/trait shape | complete transaction/current-head compile and failure matrix | Security review | yes |
| Delivery effect traits | compile-time lane separation plus intent/outcome, fixed-test, scope-before-commit, and unknown-outcome matrices | Engineering | yes |
| Task lifecycle trait | compile-time exact admit/transition/result/load separation, typed failure/replay evidence, and no raw event/SQL/cache surface | Engineering | yes |
| Core head separation | append-only Reflection tails move journal head without rewriting core head/result/state | Architecture review | yes |
| Reflection queue trait | exact known-Task pending/claim/failure/retry/degraded/load operations; no discovery or provider execution | Engineering | yes |
| Reflection history | bounded keyset pages, digest completeness, cursor/limit/stale-page rejection, no raw evidence | Security review | yes |
| Reflection candidate | closed candidate kinds, exact history binding, reserved-command namespace denial, no update/delete/promotion/core transition | Security review | yes |
| Lease-bound writer | complete spec/intent/workspace/lease/fence mutation matrix and generic-writer non-wiring proof | Security review | yes |
| Dependency direction | Cargo metadata proves only Contracts plus Task Domain closed values, with no adapter/orchestrator/I/O/MCP dependency | Architecture review | yes |
| Full Rust verification | workspace format, lint, and tests | Engineering | yes |

## Change Policy

Mission, trait signatures, error meaning, evidence meaning, history digest
contract, candidate authority, or dependency direction changes require a
versioned amendment, SPEC/ADR trace, architecture review, and explicit
responsible-user approval.

## Amendment History

| Version | Date | Decision reference | Summary | Approver |
|---|---|---|---|---|
| 1.0 | 2026-07-29 | SPEC-002 v3, ADR-004/006 | Inbound gateway plus four abstract outbound ports | User |
| 1.1 | 2026-08-01 | SPEC-002 v13, ADR-015, TASK-017 | Complete typed gateway peer/request/reply signature; contracts-only dependency retained | User MVP-3 execution directive |
| 1.2 | 2026-08-01 | SPEC-002 v14, ADR-015 review amendment, TASK-017 | Component-free Rust-core Gateway service error; external port attribution retained only for adapters/store | User MVP-3 execution directive |
| 1.3 | 2026-08-01 | SPEC-002 v15, ADR-016, TASK-018 | Complete typed Store transaction/current-head boundary and Store-specific failure semantics | User MVP-3 execution directive |
| 1.4 | 2026-08-02 | SPEC-002 v22, ADR-018, TASK-020 | Explicit mutable current-head query and live physical receipt semantics without exposing a driver | User MVP-3 execution directive |
| 1.5 | 2026-08-05 | SPEC-002 v25, ADR-021, TASK-032 | Typed delivery-ledger, bounded workspace/Git, and fixed-test traits while retaining contracts-only dependency direction | User approval in preceding implementation window |
| 1.6 | 2026-08-05 | SPEC-002 v26, ADR-021 clarification, TASK-032 | Record the approved typed `DeliveryCodexPort` specialization; freeze generic `CodexPort` outside the production delivery composition | User approval of typed delivery contracts/ports in preceding implementation window |
| 1.7 | 2026-08-05 | SPEC-002 v26, ADR-022, TASK-033 | Add exact snapshot, Graphify analysis, PostgreSQL Memory, and retrieval ports while retaining contracts-only dependencies | User TASK-033 direction |
| 1.8 | 2026-08-09 | SPEC-003 v3, ADR-023, TASK-038 | Add neutral Task lifecycle operations, an explicit Task Domain `TaskState` dependency, and lease-bound sole-writer requests for bounded MCP Submit/Status | User TASK-038-first direction |
| 1.9 | 2026-08-11 | Issue #9 decision freeze, ADR-024, GH-9 | Add independent bounded Reflection queue/history/candidate Ports, core-head anchoring, and provider-neutral Port-first/MCP-second governance without adding a scheduler, provider process, or second truth | Responsible user architecture amendment |