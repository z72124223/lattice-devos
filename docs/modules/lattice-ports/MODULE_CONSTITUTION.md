---
module_id: lattice-ports
name: LATTICE I/O Ports
version: 2.0
status: active
owner: LATTICE maintainers
last_reviewed: 2026-08-15
---

## Mission

Define the abstract Rust traits through which orchestration reaches the gateway,
sole product-code writer, read-only knowledge lane, untrusted research lane,
typed physical control store, durable delivery ledger, bounded workspace/Git
lane, fixed-test lane, authoritative Task lifecycle repository lane, and the
narrow foreman snapshot append/replay boundary.

## Non-Goals

- Select or start OpenClaw, Codex, Graphify, Hermes, or PostgreSQL.
- Perform I/O, decide policy, own workflow order, or define domain transitions.
- Select a workspace, construct a command line, choose a test, create a Git
  commit, or interpret an MCP call.
- Manufacture PostgreSQL durability or external component compatibility; a
  concrete Store may return a structurally classified physical receipt whose
  durability still requires its own implementation evidence.

## Owned Data

- Port traits, external-port errors, and a component-free inbound
  `GatewayServiceError`.
- Bounded `TaskLifecycleError` and replay-derived `TaskLifecycleEvidence`
  transport values. Task Domain remains the semantic owner of `TaskState` and
  transition legality.
- No runtime, durable, product, credential, or provider-session data.
- The foreman port error boundary and typed append/load method shape only; it
  owns neither snapshot state nor persistence.

## Public Contracts

- `GatewayService` accepts server-derived peer context plus one complete typed
  request and returns a typed bound reply; codec errors remain outside the
  service call.
- `GatewayService` returns `GatewayServiceResult`: Rust-core routing or
  reply-binding failures cannot be attributed to an external component.
- `DeliveryCodexPort` is the sole typed product-code mutation contract used by
  Orchestrator 2.3. Its request binds the complete Task Spec digest, durable
  intent, prepared workspace, and current Writer Lease/fencing evidence. The
  earlier generic `CodexPort` remains source-compatible
  only for pre-delivery consumers; it is frozen and cannot be wired as a
  second production writer beside the typed delivery lane.
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
  Task-Ledger-owned autonomy receipt, appends one caller-validated state
  transition, records one result digest, and loads one replay-derived
  authoritative lifecycle projection. The neutral projection carries one
  closed `TaskLifecycleAutonomyEvidence` sum type: `Unadmitted`,
  `HistoricalOptional(Option<receipt>)`, or `RequiredComplete(receipt)`. It exposes no
  SQL, database client, event fragment, arbitrary payload, cache, or alternate
  state mutation, and it does not decide transition or receipt legality.
- `ForemanCoordinationPort` appends one validated snapshot through a stable
  command and exact Writer authority, or loads verified snapshots for fresh
  reconstruction. It exposes no SQL, dashboard, diagnostic JSON, transcript,
  secret, generic event fragment, or independent current-state mutation.
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

## Invariants

1. This crate depends only on `lattice-contracts` and Task Domain 2.2. The Task
   Domain dependency is limited to the closed `TaskState` value used by
   `TaskLifecyclePort`; no validation/planning implementation enters Ports.
2. OpenClaw is an inbound gateway client, never a second control core or an
   outbound provider selected by orchestration.
3. Traits expose no concrete database, filesystem, or process type.
4. A port cannot return another lane's evidence type.
5. Port errors are explicit and unknown outcomes never imply success.
6. No adapter calls another adapter through these traits.
7. `GatewayService` never returns OpenClaw-produced evidence as a substitute
   for a Rust-core routing reply.
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
12. Delivery, workspace/Git, and test traits expose no concrete driver,
    process, filesystem handle, command line, SQL, credential, or MCP type.
13. Port methods do not own effect order. Only the injected orchestrator may
    sequence intent, Codex, workspace/test/Git, and terminal persistence.
14. Graph-memory ports expose no concrete Git, process, filesystem, JSON,
    database, driver, transaction, staging-directory, or Graphify CLI type.
15. Snapshot materialization, analysis, persistence, and retrieval are
    distinct effects. A failed or uncertain earlier port cannot be represented
    as a later success.
14. The workspace/Git port cannot commit before receiving request-bound passing
    scope and fixed-test evidence.
15. Unknown ledger, Codex, workspace, test, or Git outcome never becomes a
    successful port result.
16. The Task lifecycle port owns no Task Domain legality or workflow
    order. Orchestrator supplies validated typed transitions/effects and the
    adapter delegates append/replay to Task Ledger semantics.
17. A Task-control status load performs no workspace, Codex, verification,
    Git, Graphify, Hermes, or Memory effect and returns no raw event,
    diagnostic, prompt, command, path, SQL, secret, lease/fence, or child
    output.
18. No task port accepts caller-selected actor authority, project path,
    verification command, writer lease, fencing token, or Codex thread.
19. Writer Lease repository semantics/traits remain owned by Writer Lease 1.1;
    this crate neither duplicates nor wraps them into a second authority.
20. `TaskLifecycleAutonomyEvidence` is only a transport of Task Ledger verified
    state. Its closed variants prevent a required profile without a receipt,
    a missing profile with a receipt, or an unadmitted stream from being
    represented as admitted. Ports does not parse `TASK_CREATED.action` or
    select a profile.
21. `record_autonomy_receipt` transports an already bound Writer authority to
    the adapter and returns replay-derived evidence. Ports cannot build,
    classify, hash, persist, or independently validate the receipt.
22. A required profile with a missing receipt is Ledger reconciliation and has
    no successful `TaskLifecycleEvidence` representation. `admit` may expose
    only the bounded `TaskLifecycleAdmission::PendingRequiredReceipt` result so
    the exact sequence-2 append can reconcile it; normal `load`, transition,
    dispatch, and Status must reject it. It is never normal Draft success or
    terminal Status. Historical optional evidence may contain `None`; no caller
    Boolean or independent `Option` selects the rule.

## Allowed Dependencies

- `lattice-contracts`.
- `lattice-task-domain` 2.2 only for the closed `TaskState` representation in
  Task lifecycle request/evidence signatures.
- `lattice-foreman-state` 1.2 only for validated snapshot/projection values.
- Rust standard library.

## Forbidden Dependencies

- Concrete adapters, Orchestrator, policy implementation, database drivers, network/process
  clients, model SDKs, credentials, and product repositories.

## Failure, Compatibility, And Migration

Rejected calls and exhausted/unknown outcomes return typed errors. Version 1.6
explicitly records the already-approved `DeliveryCodexPort` specialization
used by Orchestrator 2.1 and freezes the earlier generic `CodexPort` outside
the production delivery composition. This is one logical writer lane with two
versioned interface shapes, never two runtime writers. Version 1.5 preserves
every 1.4 signature and adds typed delivery-ledger, workspace/Git, and
fixed-test traits for Orchestrator 2.1. Version 1.4
makes `current_head` explicitly mutable for synchronous adapters and permits
the unchanged typed receipt to carry its Contracts-owned live/durability
classification; no driver or concrete connection enters the trait. Version 1.3
replaces nominal `append(AppendCommand) -> ControlStoreEvidence` with typed
`transact(StoreTransactionRequest) -> StoreTransactionReceipt` and an exact
physical-head query. Store failures use a Store-specific typed error; no trait
claims durability or domain authority. Version 1.2
separates the inbound Rust-core `GatewayServiceError` from component-attributed
external `PortError` values and returns `GatewayServiceResult<GatewayReply>`.
Version 1.1 replaced the nominal `GatewayCommand -> GatewayEvidence` signature
with `(GatewayPeerContext, GatewayRequest) -> GatewayReply`; physical codec and
authentication remain outside this crate. Later signature or semantic changes
require a versioned amendment and coordinated consumer migration.

Version 1.8 adds the neutral Task lifecycle repository boundary and lease-bound
Codex-delivery request shape for Orchestrator 2.3. It retains the contracts-only
core boundary except for an explicit one-way Task Domain 2.2 dependency on the
closed `TaskState` value. It adds no database, domain transition implementation,
lease, process, filesystem, Git, MCP, actor-authentication, or workflow
implementation.

Version 1.9 records the internal autonomy-receipt method and projection as a
versioned lifecycle contract and adds the neutral closed autonomy-evidence sum type.
Task Ledger 2.3 remains the semantic owner; adapters fail closed on required
profile progress or Status without the receipt. No public MCP field, SQL type,
profile selector, or caller authority is added.

## Acceptance Gates

| Gate | Evidence | Owner | Required for merge |
|---|---|---|---|
| Port contract tests | `cargo test -p lattice-ports` | Engineering | yes |
| Store error/trait shape | complete transaction/current-head compile and failure matrix | Security review | yes |
| Delivery effect traits | compile-time lane separation plus intent/outcome, fixed-test, scope-before-commit, and unknown-outcome matrices | Engineering | yes |
| Task lifecycle trait | compile-time exact admit/transition/result/load separation, typed failure/replay evidence, and no raw event/SQL/cache surface | Engineering | yes |
| Lease-bound writer | complete spec/intent/workspace/lease/fence mutation matrix and generic-writer non-wiring proof | Security review | yes |
| Dependency direction | Cargo metadata proves only Contracts plus Task Domain's closed `TaskState`, with no adapter/orchestrator/I/O dependency | Architecture review | yes |
| Full Rust verification | workspace format, lint, and tests | Engineering | yes |

## Change Policy

Mission, trait signatures, error meaning, or dependency direction changes
require a versioned amendment, SPEC-002 trace, architecture review, and user
approval.

## Amendment History

| Version | Date | Decision reference | Summary | Approver |
|---|---|---|---|---|
| 2.0 | 2026-08-21 | SPEC-006 v3, ADR-024/025, TASK-079/087 | Add a narrow typed foreman append/replay port; no SQL, dashboard or second truth surface | Fixed-foreman delegation |
| 1.0 | 2026-07-29 | SPEC-002 v3, ADR-004/006 | Inbound gateway plus four abstract outbound ports | User |
| 1.1 | 2026-08-01 | SPEC-002 v13, ADR-015, TASK-017 | Complete typed gateway peer/request/reply signature; contracts-only dependency retained | User MVP-3 execution directive |
| 1.2 | 2026-08-01 | SPEC-002 v14, ADR-015 review amendment, TASK-017 | Component-free Rust-core Gateway service error; external port attribution retained only for adapters/store | User MVP-3 execution directive |
| 1.3 | 2026-08-01 | SPEC-002 v15, ADR-016, TASK-018 | Complete typed Store transaction/current-head boundary and Store-specific failure semantics | User MVP-3 execution directive |
| 1.4 | 2026-08-02 | SPEC-002 v22, ADR-018, TASK-020 | Explicit mutable current-head query and live physical receipt semantics without exposing a driver | User MVP-3 execution directive |
| 1.5 | 2026-08-05 | SPEC-002 v25, ADR-021, TASK-032 | Typed delivery-ledger, bounded workspace/Git, and fixed-test traits while retaining contracts-only dependency direction | User approval in preceding implementation window |
| 1.6 | 2026-08-05 | SPEC-002 v26, ADR-021 clarification, TASK-032 | Record the approved typed `DeliveryCodexPort` specialization; freeze generic `CodexPort` outside the production delivery composition | User approval of typed delivery contracts/ports in preceding implementation window |
| 1.7 | 2026-08-05 | SPEC-002 v26, ADR-022, TASK-033 | Add exact snapshot, Graphify analysis, PostgreSQL Memory, and retrieval ports while retaining contracts-only dependencies | User TASK-033 direction |
| 1.8 | 2026-08-09 | SPEC-003 v3, ADR-023, TASK-038 | Add neutral Task lifecycle operations, an explicit Task Domain `TaskState` dependency, and lease-bound sole-writer requests for bounded MCP Submit/Status | User TASK-038-first direction |
| 1.9 | 2026-08-15 | SPEC-002 v35, ADR-011/019, TASK-050 | Version the internal autonomy-receipt/profile projection boundary and required-profile fail-closed transport semantics without changing public MCP | User-approved TASK-050 repair amendment |
