---
module_id: orchestrator-runtime
name: Orchestrator and Runtime Port
version: 2.7
status: active
owner: LATTICE maintainers
last_reviewed: 2026-08-25
---

## Mission

Provide the pure Rust effect coordinator that routes admitted requests and
orders policy, durable intent, runtime, workspace, verification, Git, terminal
evidence, review, stop, reconciliation, and improvement workflows only through
injected ports. `latticed`, not this module, is the application entry and
composition root.
It also owns the pure replay-first foreman checkpoint effect order; it does not
own the snapshot, Ledger, Git observation, or Writer authority.

## Non-Goals

- Maintain task truth outside Task Ledger/PostgreSQL.
- Parse raw OpenClaw frames or authenticate an IPC peer.
- Parse MCP stdio, select concrete adapters, load credentials/configuration, or
  provide a second application entry.
- Let an adapter append events, call another adapter, or own workflow order.
- Call PostgreSQL, a process, filesystem, test command, or Git implementation
  directly.
- Bypass Policy on any project-selectable, general, protected, approval-bound,
  or externally effective task; or bypass scope, writer lease, review, or
  release gates.

## Owned Data

- Command-to-workflow routing and call order.
- Transaction/effect intent and outcome coordination.
- Timeout, stop, cancellation, reconciliation, and daemon-epoch ordering.
- Pure scripted workflow scenarios and typed call-order state; no concrete
  adapter, transport, driver, process, filesystem, test, or Git state.
- Versioned, explainable autonomy-control recommendation/receipt types for an
  already agreed task boundary. The recommendation is not durable task truth,
  and Orchestrator does not own the canonical receipt subject or its hashes.
- Pure coordination projections plus data-only dispatch and archive decisions;
  neither the snapshot nor decision is durable truth or execution authority.

Task Domain owns Task Spec/state legality; Task Ledger/PostgreSQL owns durable
truth; Policy owns decisions; Gateway IPC owns protocol; Approval Verifier owns
approval authority; Writer Lease owns fencing; Guardian owns activation.

## Public Contracts

- Implement the typed `GatewayService` after codec/peer admission.
- Route submit/plan/status/normal approval/rejection/task-stop only.
- Revalidate Task Spec 2.1 through Task Domain before any task creation.
- For MCP Task Submit, accept only the composition-built
  `CONTROLLED_CODEX_CANARY` Task Spec and fixed server-derived peer; never
  interpret raw model text or construct ingress authority.
- Preserve the one Task Spec digest across Gateway, Task Ledger,
  Writer Lease, Codex, verification/Git, status, and downstream evidence.
- Persist intent before an external effect and outcome after it.
- For controlled task execution, call only injected boundaries in this order:
  TaskCreated/admission audit, autonomy recommendation, real Writer Lease
  acquire/current-head when the recommendation can proceed, Task-Ledger-owned
  autonomy receipt, bounded workspace, Codex, scope verification, fixed test,
  Git commit, lease release or reconciliation, durable outcome/status, then
  the configured Graphify/Hermes/Memory continuation. The first failed or
  uncertain step suppresses every later call.
- For typed delivery, call only injected ports in this order: durable intent,
  bounded workspace preparation, Codex, workspace changed-path inspection,
  fixed test, Git commit, durable terminal outcome/receipt. Codex is
  unreachable before workspace evidence, and Git is unreachable unless scope
  and test evidence pass.
- Load delivery status only through the injected delivery-ledger port.
- Load Task Status only through the injected Task-control Ledger port, replay
  Task Domain transitions, verify the exact current head/checkpoint, and return
  the public allowlisted projection. Status performs no external effect.
- For graph memory, call only injected ports in this order: exact tracked
  snapshot, Graphify analysis, pure graph validation/normalization, durable
  analysis/record persistence, exact-snapshot deterministic retrieval/audit,
  and typed receipt/status projection.
- Stop on the first failed gate; ambiguous effects enter reconciliation.
- Revoke writer authority before verification/review and require a separate
  exact merge approval before integration.
- Recommend only the existing governed Codex writer or no model; return a
  typed user decision for missing preapproval, new authority, or high-risk/
  irreversible work without invoking a model or changing lifecycle state.
- Project work/completion evidence and recommend a round only for unique
  `READY`, dependency-complete, resource-valid, conflict-free work. Recompute
  after completion registration and recommend `ARCHIVE`/`RETAIN` without
  performing either action.
- For a foreman checkpoint, replay the exact intent first. Only a new intent may
  observe server binding/Git and acquire Writer authority, then append under the
  fence and release after known append success. Unknown append stops before
  release; unknown release stops without repeating append.

## Invariants

1. Orchestrator is the only normal workflow event appender.
2. PostgreSQL remains the sole live durable command/event truth.
3. One current writer lease and one Codex process/thread owner exist per task.
4. Gateway routing cannot manufacture approval, writer, database, or Guardian
   authority.
5. `STOP_REQUESTED` precedes interrupt/reconciliation/lease release and is not
   terminal stop evidence.
6. First failed gate stops progress; unknown outcome never implies success.
7. Fake scenarios perform no model, network, credential, product, deployment,
   publication, or protected-release effect.
8. The crate has no concrete adapter, MCP, database driver, process, filesystem,
   test runner, or Git dependency.
9. The first failed or uncertain delivery stage stops later port calls;
   reconciliation cannot become success.
10. A compatibility caller reaches the same coordinator and cannot introduce a
    second call order or durable status source.
11. Graphify is unreachable until exact commit/tree/source-manifest evidence
    exists. Persistence is unreachable until complete output validates, and
    retrieval is unreachable until the same analysis is durably complete.
12. Timeout, malformed/partial output, provenance mismatch, ranking failure,
    database ambiguity, or changed binding stops later calls and never becomes
    a graph-memory success.
13. A product-code writer is unreachable without a live Task-Spec-bound
    Writer Lease authority receipt plus independently loaded matching current
    head and fencing token. Fake, synthetic, expired, suspect, stale, or
    receipt-only authority is rejected.
14. Heartbeat/currentness is checked at each mutation boundary; verification
    and Git cannot accept evidence from another lease/fence/worktree/process.
15. Task lifecycle, exact idempotency, fixed-profile audit, and
    status have no in-memory authoritative fallback.
16. Fresh-process Task Status replays PostgreSQL and cannot rerun Codex,
    verification, Git, Graphify, Hermes, or Memory.
17. The fixed MCP actor may submit/status only the approved canary template;
    it cannot plan, approve, reject, stop, acquire a lease directly, or control
    a writable Codex thread.
18. The canary cannot fabricate a live Project Registry fact or be generalized
    into project selection/free-form work. Those surfaces require the normal
    independently current Registry and Policy composition first.
19. Autonomy-control recommendation is non-authoritative: it cannot schedule,
    create, persist, approve, or transition a task. Only Task Ledger can build,
    hash, order, and verify its durable receipt after an existing binding.
20. A missing or rejected required receipt stops every task transition and
    external effect. Orchestrator cannot turn a `PendingRequiredReceipt`
    lifecycle projection into normal Draft success.
21. Unknown/blocked/incomplete evidence, duplicate work/report IDs,
    undeclared/duplicate/self dependency, invalid resources, and resource
    collision fail closed without dispatch or archive from an invalid round.
22. Dispatch/archive values are data only. They cannot create/control a window
    or process, reserve resources, invoke Codex, mutate files/PostgreSQL, access
    network/credentials, or bypass the governed execution path.
23. Foreman exact retry performs no new observation or Writer effect. No unknown
    append/release outcome is converted to success or duplicate append.

## Allowed Dependencies

- `lattice-contracts`, `lattice-task-domain`,
  `lattice-codebase-memory`, `lattice-ports`, and `lattice-writer-lease` 1.1
  public APIs.
- `lattice-foreman-state` 1.3 closed checkpoint/snapshot/projection values only.
- Injected Registry, Ledger, approval, workspace, runtime, verification,
  review, integration, clock, and ID ports.

## Forbidden Dependencies

- Direct OpenClaw SDK/raw IPC parser, direct provider/model clients, hidden
  filesystem/Git mutation, adapter-to-adapter calls, reviewer mutation ports,
  and direct Guardian trust-root access.
- MCP libraries/transports, concrete PostgreSQL/Codex/workspace/test/Git
  adapters, database/process/filesystem/Git clients, credentials, and product
  repositories.

## Failure, Compatibility, And Migration

Intent failure blocks an action. Unknown commit/effect outcome produces
reconciliation. Version 2.1 adds the typed TASK-032 delivery order while
removing application-entry ambiguity: `latticed` owns composition and this
module remains injected and I/O-free. Port/schema changes require compatibility
adapters and version evidence. The V1 Node fake remains characterization only.

Version 2.3 adds bounded typed Submit/Status and the formal lease-governed
canary order. It does not add a concrete repository, MCP parser, process,
filesystem, Git, database driver, task cache, rate store, second application
entry, or alternate writer path. Existing delivery/graph-memory callers remain
compatible but cannot be used to bypass the new Task Spec/lease binding.

Version 2.4 adds a pure, versioned decision classifier for already agreed task
intent. It preserves all Task Spec, Policy, Ledger, lease/fence, MCP, and
composition boundaries and does not claim model or scheduler control.

Version 2.5 adds the pure TASK-055 coordination gate over typed snapshots. It
adds no concrete scheduler, adapter, I/O, MCP, or durable authority.

Version 2.6 makes the Task-Ledger-owned autonomy receipt an explicit gate after
admission and before any task transition or external effect. It removes
canonical receipt/hash ownership from Orchestrator while preserving the pure
classifier, dependency direction, and public MCP surface.

## Acceptance Gates

| Gate | Evidence | Owner | Required for merge |
|---|---|---|---|
| Gateway routing | closed action and exact binding tests | Engineering | yes when implemented |
| Controlled submit | fixed actor/template, complete Task Spec 2.1 validation, exact idempotency/audit, and digest-unity matrices | Engineering | yes |
| Writer authority order | fixed admission -> real lease/current head -> workspace -> Codex -> verification -> Git -> release/outcome, with stale/fake/synthetic substitution denial | Security review | yes |
| Required autonomy order | admission -> recommendation -> required receipt -> transition/effect, with missing/late/duplicate/unknown profile suppression | Security review | yes |
| Task status replay | fresh-process PostgreSQL projection equality and zero external-effect calls | Engineering | yes |
| Call order | intent/effect/outcome/stop/review tests | Engineering | yes when implemented |
| Delivery call order | intent -> workspace prepare -> Codex -> scope -> fixed test -> Git -> outcome/receipt, with first-failure call suppression | Engineering | yes |
| Fake scenarios | success/failure/timeout/cancel/malformed/ambiguous tests | Engineering | yes when implemented |
| No external call | injected-only dependency inspection | Security review | yes |
| Coordination round | projection, identity/dependency/evidence/resource conflict, next-round, and archive matrices | Engineering | yes |
| End-to-end | one offline task with restart/replay evidence | MVP-1 exit | yes |

## Change Policy

Gate order, event ownership, writer authority, gateway routing, port contracts,
stop/reconciliation semantics, or agent limits require a versioned amendment,
SPEC/ADR update, architecture review, and responsible-user authorization.

## Amendment History

| Version | Date | Decision reference | Summary | Approver |
|---|---|---|---|---|
| 2.7 | 2026-08-25 | SPEC-009, ADR-027, TASK-105 | Order replay-first foreman checkpoint, Writer acquire, fenced append and known-success release with explicit unknown-outcome stops | Sole-foreman delegation |
| 1.0 | 2026-07-29 | SPEC-001, ADR-001/002 | Initial Node deterministic control loop | Current user task |
| 2.0 | 2026-08-01 | SPEC-002 v13, ADR-004/005/006/007/015 | Rust routing, transaction/effect/stop/reconciliation boundary | User MVP-3 execution directive |
| 2.1 | 2026-08-05 | SPEC-002 v25, ADR-021, TASK-032 | Pure injected delivery ordering and explicit separation from `latticed` composition/MCP/concrete adapters | User approval in preceding implementation window |
| 2.2 | 2026-08-05 | SPEC-002 v26, ADR-022, TASK-033 | Pure exact snapshot -> Graphify -> validate -> PostgreSQL Memory -> retrieval ordering | User TASK-033 direction |
| 2.3 | 2026-08-09 | SPEC-003 v3, ADR-023, TASK-038 | Bounded Gateway Submit/Status, one Task Spec digest, PostgreSQL task control, and real Writer Lease/fencing before Codex | User TASK-038-first direction |
| 2.4 | 2026-08-12 | Autonomous execution-control user direction | Pure versioned intent classification, safe writer/verification recommendation, and non-durable state receipt | Current user task |
| 2.5 | 2026-08-13 | TASK-055 | Pure fail-closed work/evidence projection, next-round dispatch, and archive recommendation without I/O or execution authority | Current user task |
| 2.6 | 2026-08-15 | SPEC-002 v35, ADR-011/019, TASK-050 | Gate controlled progression on the Task-Ledger-owned required receipt and retain only the pure autonomy recommendation in Orchestrator | User-approved TASK-050 repair amendment |
