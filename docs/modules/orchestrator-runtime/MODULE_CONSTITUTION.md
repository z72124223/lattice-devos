---
module_id: orchestrator-runtime
name: Orchestrator and Runtime Port
version: 3.1
status: active
owner: LATTICE maintainers
last_reviewed: 2026-08-26
---

## Mission

Provide the pure Rust effect coordinator that routes admitted requests and
orders policy, durable intent, runtime, workspace, verification, Git, terminal
evidence, review, stop, reconciliation, and improvement workflows only through
injected ports. `latticed`, not this module, is the application entry and
composition root.
It also owns the pure replay-first foreman checkpoint effect order; it does not
own the snapshot, Ledger, Git observation, or Writer authority.
Version 2.9 additionally owns the one-call create-only order for a verified
general-task intake; that order uses a separate narrow port and deliberately
cannot reach Task-Spec lifecycle or execution authority.
Version 3.0 additionally owns the pure type-staged managed-attempt workflow,
retained-attempt restart reconciliation, and bounded stall recovery. Concrete
PostgreSQL, Node/App Server, workspace snapshot, clock, and runtime composition
remain injected adapters outside this module.
Version 3.1 clarifies terminal Writer cleanup: a separately durable closed
`BLOCKED` or `FAILED` decision may release its exact Writer only after no live
provider effect remains; a retained no-effect closure by itself grants no
release authority.

## Non-Goals

- Maintain task truth outside Task Ledger/PostgreSQL.
- Parse raw OpenClaw frames or authenticate an IPC peer.
- Parse MCP stdio, select concrete adapters, load credentials/configuration, or
  provide a second application entry.
- Let an adapter append events, call another adapter, or own workflow order.
- Call PostgreSQL, a process, filesystem, test command, or Git implementation
  directly.
- Treat create-only general intake as a Policy allow decision or use it to
  bypass Policy on any progression, protected, approval-bound, or externally
  effective task; or bypass scope, writer lease, review, or release gates.

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
- Type-state markers for managed starting versus exact-started execution,
  managed restart/stall outcomes, and the final
  `AwaitingMergeApproval` recommendation. These are pure coordination values,
  not Task-Ledger state or durable authority.

Task Domain owns Task Spec/state legality; Task Ledger/PostgreSQL owns durable
truth; Policy owns decisions; Gateway IPC owns protocol; Approval Verifier owns
approval authority; Writer Lease owns fencing; Guardian owns activation.

## Public Contracts

- Implement the typed `GatewayService` after codec/peer admission.
- Route submit/plan/status/normal approval/rejection/task-stop only.
- Revalidate Task Spec 2.1 through Task Domain before any executable
  Task-Spec lifecycle creation or progression.
- For MCP Task Submit, accept only the composition-built
  `CONTROLLED_CODEX_CANARY` Task Spec or one composition-verified general-task
  binding and fixed server-derived peer. Natural-language objective text is
  validated and retained outside Orchestrator; this module never interprets it
  as a command or constructs project/ingress authority.
- For create-only general Task Submit, receive only a bounded
  `GeneralTaskIntakeRequest` with the shared one-to-64-byte secret-free ingress
  key plus `TaskIntakeLifecyclePort`; call only `admit`
  and return its exact `DRAFT`/no-result admission. Exact replay uses that same
  call. The binding and evidence have no Task Spec, currency, TaskKind, risk,
  autonomy, Policy, Writer Lease, workspace, Codex/model, verification, Git,
  payment, network, merge, deployment, or release surface and therefore
  cannot start execution.
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
- For one managed successor, use `run_managed_workflow` (or its exported staged
  functions) as the only normal progression owner. The fixed order is:
  successor admission/replay; durable autonomy; `DRAFT ->
  AWAITING_EXECUTION_APPROVAL`; current closed execution-authority validation;
  exact writer acquire/current/assert; `AWAITING_EXECUTION_APPROVAL ->
  PREPARING`; model availability preflight; current-authority recheck; atomic
  attempt claim; thread and turn start
  acceptance; durable exact matching `turn/started`; `PREPARING -> EXECUTING`;
  execution observations; exact terminal; independent snapshot preparation;
  durable owner-typed artifact; independent verification; `EXECUTING ->
  VERIFYING -> REVIEWING -> AWAITING_MERGE_APPROVAL`; then known writer release.
- `prepare_managed_attempt`, `confirm_managed_exact_start`, and
  `finish_managed_attempt` expose the same type-staged boundary for composition
  that must perform non-managed work between stages. A caller cannot obtain a
  `ManagedExecutingAttempt` from thread/turn RPC acceptance alone.
- On fresh-process replay of retained worker IDs, call only read exact thread,
  read exact turn, resume exact turn, and reconcile exact turn, in that order;
  an exact terminal short-circuits. Never start a replacement thread or turn
  from the restart path.
- Stall recovery uses the Foreman State watchdog's closed reason, reconciles
  retained IDs first, then (only when unresolved) interrupts the exact turn,
  durably records interrupt request and exact terminal, and returns the bounded
  retry decision. The coordinator never opens the retry itself.
- A verification pass recommends only `AWAITING_MERGE_APPROVAL`; it cannot
  merge, complete, push, deploy, or release. Verification failure and all
  uncertain/retry/restart paths retain the exact writer lease and fence while
  reconciliation or bounded repair remains pending. A separately durable
  closed `BLOCKED` or `FAILED` decision may release the exact Writer only
  after retained provider effect has been terminally reconciled or proven
  absent.

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
17. The fixed MCP actor may submit/status the approved canary template and may
    create/status a separately typed general intake only after composition has
    supplied an independently current Project Registry binding. It cannot
    plan, specify, approve, reject, stop, acquire a lease, or control a writable
    Codex thread through either intake surface.
18. The canary profile cannot fabricate a Registry fact or be generalized into
    free-form work. General intake is a distinct binding/port and remains
    pre-specification; it cannot be converted to `SubjectBinding` or enter
    Policy, approval, Writer Lease, delivery, or execution.
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
23. Foreman exact retry performs no new Git observation, Writer acquire, or
    Ledger append. It may read only the current Writer authority and issue the
    deterministic release when the replayed checkpoint's retained authority
    receipt digest matches exactly; this is the sole reconciliation exception.
    No unknown append/release outcome is converted to success or duplicate append.
24. General-task creation is exactly one narrow `admit` call returning a
    `DRAFT`/no-result intake, or its exact replay. The port cannot represent an
    autonomy receipt, TaskKind, risk class, currency, transition, or result; a
    different binding is rejected.
25. General-task creation cannot be reused as a plan, specification, ticket,
    approval, execution, writer, payment, external-action, merge, deployment,
    or release coordinator. Every such successor remains separately governed.
26. General intake does not invoke or bypass Policy because its separate port
    cannot express a Task Spec, progression, result, or external effect. Any
    later specification, project-affecting progression, execution, or external
    effect must enter the normal Task Domain, current Registry, Policy,
    approval, lease/fence, and downstream gates as a separately governed
    operation.
27. Managed model availability is checked before current-authority assertion
    and atomic attempt claim. An unavailable allowlisted model fails closed
    without consuming an active-attempt slot or silently selecting another
    model.
28. A managed task remains `PREPARING` after thread and turn start acceptance.
    Only a durably recorded exact matching in-progress `turn/started`
    observation permits the `PREPARING -> EXECUTING` transition.
29. Managed terminal observation is not task success. It must be durable before
    independent preparation, whose Artifact-Store-owned evidence must be
    durably receipt-matched before verification.
30. Managed verification pass progresses only through `VERIFYING` and
    `REVIEWING` to `AWAITING_MERGE_APPROVAL`. No managed coordinator reaches
    `MERGING`, `COMPLETED`, push, deployment, publication, or release.
31. After writer acquisition, every ambiguous, reconciliation, restart, stall,
    or still-repairable failure retains the exact lease/fence. Writer release
    occurs only after a known durable transition to `AWAITING_MERGE_APPROVAL`,
    or after a separately durable closed decision transitions the task to
    `BLOCKED` or `FAILED` and exact terminal/no-provider-effect evidence proves
    that no provider effect remains live. An attempt closure alone, an
    unresolved Writer mismatch, or retry-budget arithmetic without that
    durable decision never authorizes release.
32. Retained-attempt restart performs no start RPC. It reconciles exact stored
    provider identities first and never turns an unresolved or ambiguous read
    into a duplicate thread/turn.
33. Stall is not elapsed time alone. Only a closed Foreman State watchdog reason
    can enter recovery; exact interrupt must be followed by a durable exact
    interrupted/failed terminal before a retry is allowed.
34. Repair retries preserve the task reference, increment attempt/fence through
    semantic-owner inputs, and are capped by the injected `WorkerBudget`.
    Exceeding two repair retries returns the closed exhausted decision.

## Allowed Dependencies

- `lattice-contracts`, `lattice-task-domain`,
  `lattice-codebase-memory`, `lattice-ports`, and `lattice-writer-lease` 1.1
  public APIs.
- `lattice-foreman-state` 1.3 closed checkpoint/snapshot/projection values only.
- `lattice-task-ledger` 3.0 and `lattice-artifact-store` 1.1 values only through
  the `lattice-ports` managed boundaries; Orchestrator owns neither semantic
  record construction nor persistence.
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

Version 2.9 corrects general intake to use only `TaskIntakeBinding` and
`TaskIntakeLifecyclePort::admit`. It removes the erroneous autonomy-receipt and
Task-Spec lifecycle path. Project resolution and the authoritative submission
envelope remain outside this module; Task Ledger/PostgreSQL remain durable
truth. Existing canary execution order and every protected-action gate remain
unchanged.

Version 3.0 adds the pure managed-attempt and high-level lifecycle/writer
coordinators. It preserves existing public intake/delivery behavior and adds no
concrete adapter or I/O. Runtime compositions must migrate from manual managed
task transitions to `run_managed_workflow` or the exported type-staged
functions so exact-start, artifact-before-verification, retained-ID
reconciliation, retry budget, merge separation, and writer retention cannot be
reordered.

Version 3.1 resolves the terminal-cleanup contradiction in version 3.0.
Durable `BLOCKED`/`FAILED` outcomes with exact no-live-provider evidence may
release their matching Writer after the transition; ambiguous and repairable
paths still retain it. This changes no provider, merge, push, deployment, or
release authority.

## Acceptance Gates

| Gate | Evidence | Owner | Required for merge |
|---|---|---|---|
| Gateway routing | closed action and exact binding tests | Engineering | yes when implemented |
| Controlled submit | fixed actor/template, complete Task Spec 2.1 validation, exact idempotency/audit, and digest-unity matrices | Engineering | yes |
| General create-only intake | one admit call, exact replay, binding substitution rejection, structurally Draft/no-result evidence, and compile-time absence of Task-Spec/autonomy/writer/execution ports | Engineering | yes |
| Managed lifecycle/writer order | admission, writer, authority, Draft-to-Preparing, accepted-vs-exact-start, Executing-to-AwaitingMergeApproval, verified-success release, closed Blocked/Failed cleanup, and ambiguous/repairable failure-retention matrices | Engineering | yes |
| Managed restart/stall | retained-ID read/read/resume/reconcile without start, closed stall reason, reconcile-first, exact interrupt/terminal, and two-retry budget matrices | Engineering | yes |
| Managed verification separation | durable terminal, owner-typed artifact receipt, independent verifier pass/fail, and no Merging/Completed/external release | Security review | yes |
| Writer authority order | fixed admission -> real lease/current head -> workspace -> Codex -> verification -> Git -> release/outcome, with stale/fake/synthetic substitution denial | Security review | yes |
| Controlled-canary autonomy order | admission -> recommendation -> required receipt -> transition/effect, with missing/late/duplicate/unknown profile suppression; general intake is structurally unreachable from this lane | Security review | yes |
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
| 3.1 | 2026-08-28 | SPEC-011 v1.6, independent Phase 4 recovery review | Permit exact Writer cleanup only after separately durable closed Blocked/Failed decisions with no-live-provider evidence, while preserving retention for ambiguous and repairable paths | User-authorized Phase 4 repair |
| 3.0 | 2026-08-26 | SPEC-011, ADR-028 | Add pure managed lifecycle/writer, exact-start, restart reconciliation, bounded stall/retry, artifact-backed verification, and merge-separation coordination | User-authorized Phase 4 |
| 2.9 | 2026-08-26 | ADR-023 Phase 3 P1 correction | Separate one-call general intake from Task-Spec lifecycle and remove autonomy/writer/execution classification | User-authorized Phase 3 |
| 2.7 | 2026-08-25 | SPEC-009, ADR-027, TASK-105 | Order replay-first foreman checkpoint, Writer acquire, fenced append and known-success release with explicit unknown-outcome stops | Sole-foreman delegation |
| 2.8 | 2026-08-26 | ADR-023 Phase 3 amendment | Initial general-intake coordinator; superseded before release by 2.9 because it incorrectly reused Task-Spec lifecycle/autonomy types | User-authorized Phase 3 |
| 1.0 | 2026-07-29 | SPEC-001, ADR-001/002 | Initial Node deterministic control loop | Current user task |
| 2.0 | 2026-08-01 | SPEC-002 v13, ADR-004/005/006/007/015 | Rust routing, transaction/effect/stop/reconciliation boundary | User MVP-3 execution directive |
| 2.1 | 2026-08-05 | SPEC-002 v25, ADR-021, TASK-032 | Pure injected delivery ordering and explicit separation from `latticed` composition/MCP/concrete adapters | User approval in preceding implementation window |
| 2.2 | 2026-08-05 | SPEC-002 v26, ADR-022, TASK-033 | Pure exact snapshot -> Graphify -> validate -> PostgreSQL Memory -> retrieval ordering | User TASK-033 direction |
| 2.3 | 2026-08-09 | SPEC-003 v3, ADR-023, TASK-038 | Bounded Gateway Submit/Status, one Task Spec digest, PostgreSQL task control, and real Writer Lease/fencing before Codex | User TASK-038-first direction |
| 2.4 | 2026-08-12 | Autonomous execution-control user direction | Pure versioned intent classification, safe writer/verification recommendation, and non-durable state receipt | Current user task |
| 2.5 | 2026-08-13 | TASK-055 | Pure fail-closed work/evidence projection, next-round dispatch, and archive recommendation without I/O or execution authority | Current user task |
| 2.6 | 2026-08-15 | SPEC-002 v35, ADR-011/019, TASK-050 | Gate controlled progression on the Task-Ledger-owned required receipt and retain only the pure autonomy recommendation in Orchestrator | User-approved TASK-050 repair amendment |
