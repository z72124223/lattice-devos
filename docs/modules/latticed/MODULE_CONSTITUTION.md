---
module_id: latticed
name: LATTICE Normal Composition Root
version: 3.9
status: active
owner: LATTICE maintainers
last_reviewed: 2026-08-31
---

## Mission

Provide the sole normal local composition root that constructs the pure Rust
orchestrator with concrete delivery/task adapters and exposes the bounded Codex
App MCP stdio surface. The existing `apps/lattice-runtime` package implements
this module and maps every MCP operation into the same `FullChainService` /
Orchestrator composition. Version 3.5 consumes the Store 1.23 deployment
compatibility boundary: retained duplicate pre-v7 ingress identities have no
runtime-selected winner and every shared-ingress entrypoint fails closed.
Version 3.9 composes the append-only Store-v8 runtime successor, immutable
Writer-v5 Store-v8 rebind, and Foreman-v1 Store-v8 rebind under one bounded
bootstrap gate while keeping normal MCP startup verify-only.

## Non-Goals

- Own domain transitions, policy, workflow order, durable task truth, provider
  semantics, or product-code mutation logic.
- Become a second gateway service, database authority, product writer,
  approval surface, Guardian, release controller, or deployment service.
- Interpret a natural-language objective as shell, SQL, path, credential,
  permission, configuration, provider instruction, approval, or execution
  authority; or accept caller-selected paths, commands, actor/session, lease,
  fence, or writable-thread input through MCP.
- Promote the legacy `lattice-runtime` scripted acceptance command into an
  official writer entry or let it claim MCP/tunnel provenance.

## Owned Data

- Process-lifetime composition configuration, constructed adapter instances,
  the bounded MCP stdio session, legacy fixture routing, and the verified
  fixed tunnel/profile ingress identity used to derive the one MCP actor.
- No Task Ledger, project, workspace, Git, test, Codex-thread, approval,
  credential, memory, or release truth. PostgreSQL and their domain owners
  retain durable and semantic authority.

## Public Contracts

- Construct one Orchestrator 3.0 instance with typed Contracts 1.15 / Ports 2.4
  implementations for the bounded delivery, graph-memory, and task-control
  paths.
- Through canonical `latticed`, expose exactly seven MCP tools:
  `lattice_delivery_run`, `lattice_delivery_status`, `lattice_runtime_status`,
  `lattice_delivery_reconcile`, `lattice_task_submit`, `lattice_task_status`, and
  `lattice_foreman_checkpoint`.
  Delivery Reconcile is zero-parameter and read-only: it replays the existing
  PostgreSQL receipt to distinguish a durable terminal fact from a reconciliation
  blocker, and cannot dispatch Codex, write a receipt, or reinterpret uncertainty.
  Runtime Status is read-only,
  starts no optional component, and reports their independent readiness/degradation.
  It adds only a verified durable foreman replay projection. The checkpoint
  schema contains no caller identity, arbitrary path, authority/fence,
  database, SQL or command field; its one closed dependency object may carry
  only the validated parent/child task IDs, derived child branch/worktree ID,
  exact base SHA and fixed next action needed for restart replay.
- Through the compatibility `lattice-runtime` executable, expose one non-MCP,
  read-only `runtime-health` and `receipt-state` commands. They accept the same
  fixed marker-owned PostgreSQL binding as Delivery Status. `runtime-health`
  reports control-core readiness, PostgreSQL connection availability, and the
  configured independent Graphify/Hermes mode; it always keeps
  `delivery_receipt=NOT_INSPECTED`. `receipt-state` reports only the verified
  durable receipt projection. Neither command creates or reinterprets a
  receipt, and neither alters the seven-tool MCP surface.
- Restrict the alternate `lattice-full-chain` executable to a legacy observer
  surface. It advertises only the two delivery names, rejects Delivery Run
  with a fixed code before service dispatch, permits only durable Delivery
  Status reads, and treats both task names and `lattice_foreman_checkpoint` as
  unknown under legacy and stateless MCP.
- Preserve zero-parameter closed schemas for both delivery tools. Task Submit
  accepts either the legacy closed `CONTROLLED_CODEX_CANARY` intent or one
  bounded create-only natural-language `objective` (with a general `intent`
  alias), a bounded `client_request_id`, and at most one exact `project_id` or
  `project_name` locator. Task Status requires the lowercase SHA-256 `task_ref`;
  `client_request_id` is optional only for legacy canary compatibility. Every
  variant has `additionalProperties: false` and no path/command/authority field.
  Submitting an objective grants no execution authority. In managed `ACTIVE`
  mode the supervisor may dispatch asynchronously only after the independent
  task/spec/budget-bound gate; in `DISABLED` mode general intake is create-only.
- Through canonical `latticed --hermes-launch`, expose one exact
  process-start-only Hermes lifecycle entry. It accepts no additional CLI
  arguments, reuses the production Hermes configuration and runner from the
  full-chain composition, reports only fixed redacted failures, grants no MCP
  or task authority, and owns the runner until bounded shutdown.
- Through canonical `latticed --graphify-runtime-preflight`, expose one
  read-only identity check for a separately configured, pinned Graphify
  runtime. It never starts Graphify, PostgreSQL, Hermes, or a delivery run;
  it reports only fixed configuration/identity classifications and grants no
  MCP, task, or durable-write authority.
- Through canonical `latticed --graphify-refresh`, expose one local,
  zero-argument derived-memory refresh. It reads only the process-configured
  clean Git root at its exact HEAD and persists or replays only the matching
  PostgreSQL Graphify receipt. It creates no delivery receipt, does not
  dispatch Codex or mutate Git, and grants no Hermes authority.
- Through canonical no-argument `latticed`, accept only process-owned
  `LATTICE_HERMES_MODE=TASK_ONLY|PRODUCTION`. The default and `TASK_ONLY`
  preserve the task-only composition. `PRODUCTION` configures one lazy Hermes
  owner: only `lattice_delivery_run` may activate it, before writer effects;
  Delivery Status and both Task tools perform zero Hermes activation. MCP EOF
  explicitly reaps an activated runner, and teardown ambiguity cannot exit 0.
- Through canonical `latticed`, default the process-owned
  `LATTICE_RUNTIME_INTEGRATION` setting to `CORE_ONLY`. `GRAPHIFY` adds only
  the independently verifiable Graphify projection. `GRAPHIFY_HERMES` adds
  Hermes reflection after Graphify; legacy `FULL_CHAIN` remains its alias for
  compatibility. A Graphify or Hermes failure is reported as that component's
  bounded `DEGRADED` status and error code; it never erases or replaces a
  verified PostgreSQL delivery receipt.
- Production Hermes configuration requires exactly the twelve executed-input
  settings for preparation, runtime, containment, API bearer, Codex launcher
  and home, broker run root, and deadline. Legacy
  `LATTICE_HERMES_BROKER_HELPER{,_SHA256}` values are ignored and must not be
  read, validated, echoed, or allowed to affect launch classification.
- For executable delivery/canary work, select every project, snapshot,
  repository/base, scope, verification, capability, budget, approval, prompt,
  workspace, and downstream binding from process-start composition
  configuration, never from MCP arguments. General intake may carry only an
  exact Control catalog ID or display-name locator; it can select no path,
  scope, verification, model, workspace, command, or downstream binding.
  Credentials remain adapter-private process input and never enter tool
  schemas, diagnostics, or results.
- Map `lattice_delivery_run` to the one typed delivery coordinator and map
  `lattice_delivery_status` to the same durable ledger/status projection.
- Map Task Submit/Status into the same `FullChainService` and injected
  Orchestrator used by `latticed`; do not create an MCP-specific gateway,
  queue, or workflow owner.
  Derive a closed ingress actor from verified process-start configuration;
  production Secure MCP Tunnel and local canonical acceptance use distinct
  non-substitutable actor/adapter commitments. `clientInfo` and caller identity
  fields grant no authority.
- Resolve a general-task project in two explicit layers. Control supplies only
  a uniquely matched readable `CONTROL_LOCAL_CATALOG` locator whose
  `registry_authority` remains `NONE`; it is never formal authority. The
  composition root performs a bounded live repository observation between two
  exact eligible Control catalog-projection reads and rejects eligible catalog drift while
  ignoring unrelated Control status fields, then uses
  `PostgresProjectRegistry` to resolve or register and reload the formal
  identity. Only a replay-verified current `ACTIVE` Project Registry authority
  receipt/snapshot may enter the Task Ledger binding. Missing, ambiguous,
  legacy, unreadable, drifted, suspended, or substituted projects return typed
  repairable errors; an arbitrary caller path is never accepted.
- Map Foreman Checkpoint into that same `FullChainService` and injected
  Orchestrator. The composition root may observe its fixed server binding and
  make one closed, read-only Git observation of the configured parent plus an
  exactly marker-owned dependency worktree. Git hooks and fsmonitor stay
  disabled, and the captured parent observation must pass through the
  Orchestrator callback and typed snapshot/ports; the MCP adapter cannot call
  Git directly. The composition root cannot mutate Git or Ledger state,
  acquire Writer authority, or reorder effects directly. This preserves One
  Gateway for all seven canonical tools.
- Construct the complete Task Spec 2.1 from the server-owned canary template,
  revalidate it through Task Domain, and preserve its one digest across
  Gateway, Task Ledger, Writer Lease, Codex, verification/Git, and status
  evidence.
- For general intake, construct and revalidate the Task-Ledger-owned
  `TaskSubmissionEnvelope` from the exact objective, registered-project
  authority receipt, formal stream identity, and process-owned ingress. Do not
  treat the objective itself as an execution-ready Task Spec, approval, model,
  command, or path. When the process-owned managed-foreman profile is active,
  the composition may promote that replay-verified intake exactly once into a
  server-built bounded Task Spec and schedule its successor through the typed
  Orchestrator; the immutable promotion link is not a parallel task record.
- Before constructing or reloading the formal identity used for a managed task
  claim, replay Foreman Runtime status from PostgreSQL and require a nonzero
  latest generation, exactly one `ACTIVE`, zero `BLOCKED`, zero `COMPLETED`,
  and `next_action=CONTINUE`. Every other projection returns the fixed
  `LATTICE_MANAGED_FOREMAN_NOT_ACTIVE` failure before claim or provider RPC.
- Compose the injected Writer Lease domain 1.1 repository and Task lifecycle
  port. Production task dispatch cannot use `FakeWriterLease`, synthetic
  authority, or a process-memory task/status store.
- The local Task-Spec lifecycle edge wrapper implements `TaskLifecyclePort`
  only by translating typed admit/transition/result/load calls into Task Ledger
  and `PostgresTaskLedger` public append/replay APIs. Controlled-canary
  admissions carry their exact required-profile marker, and canonical autonomy
  receipt construction/verification remains Task-Ledger-owned. A separate
  `TaskIntakeLifecyclePort` edge admits or loads only `TaskIntakeBinding`; it
  cannot transition, record a result or autonomy receipt, acquire Writer
  authority, or invoke an effect. Neither wrapper owns transition legality,
  receipt semantics, SQL/schema, alternate cache, or workflow order.
- After the preconfigured scripted delivery receipt, the run tool invokes the
  same coordinator's exact-snapshot Graphify/memory node. The status tool loads
  delivery plus exact analysis/retrieval evidence from PostgreSQL; neither tool
  gains an argument, and task tools cannot alter their delivery binding.
- The fixed canary remains `WriterOnly`: after its durable result and Writer
  Lease release, Submit/Status returns without running Graphify, Hermes, or
  Memory. General Task intake remains create-only: it atomically records the
  shared ingress claim, authoritative envelope, and one
  `GENERAL_TASK_INTAKE_V1` `TASK_CREATED` event. That event grants no Task Spec,
  accounting, autonomy, transition, result, or effect. A separately configured
  managed foreman may then consume the committed intake asynchronously, bind a
  server-built spec/approval/budget, acquire Writer authority, and invoke the
  exact Codex and verification ports. It still cannot invoke Graphify, Hermes,
  Memory, payment, push, merge, deployment, publication, or release.
  The fixed canary
  accepts only a process deadline above the 30-second cleanup
  reserve and at most 300 seconds, below its 600-second lease TTL. Managed
  general work has its own digest-bound 900-second budget, heartbeat, exact
  interruption, bounded retry, and restart-reconciliation contract.
- Retain `lattice-runtime delivery-run` only as a visibly scripted, exact
  repository-owned acceptance fixture. It rejects official Codex mode before
  identity, database, workspace, or process effects; canonical `latticed` is
  the only official writer entry.
- Keep OpenClaw as the broader normal human gateway. MCP permits the retained
  fixed canary plus bounded general create/status only; it provides no general
  plan/specification/ticket generation, approve/reject/stop, execution start,
  writable-agent control, or protected-release operation.
- Return only the allowlisted typed status projection and reconstruct it from
  PostgreSQL in a fresh process/session without rerunning external effects.
  While the managed foreman is enabled, both unpromoted and promoted general
  tasks use the closed `lattice.task.status.v4` projection; unpromoted tasks
  have no attempt/worker and expose any durable preparation blocker/next
  action. The projection adds only
  replay-verified phase, real-running flag, attempt/retry count, allowlisted
  model/reasoning, exact thread/turn IDs, last progress, blocker, verification
  and evidence digests, normalized resource observation, next action, and
  formal foreman generation/checkpoint. The snapshot boundary remains the
  Registry-compatible 159 ASCII bytes. Canary results retain compatible v2.
  A Completed projection additionally requires an existing, independently
  replayed Writer Lease project with no current authority and the fixed canary
  `1/2/2` fence/transition/command history. `Merging + result` recovery verifies
  active `1/1/1` or released `1/2/2` before any further Task Ledger mutation.
- A dedicated status-only fallback may project a fully verified historical
  `Failed`, `Rejected`, `Blocked`, or `Cancelled` stream after binary commitment
  drift. It must revalidate the closed historical ingress audit and complete
  Ledger history, write nothing, preserve exact task-reference comparison, and
  remain unreachable from Submit, resume, transition, result, and effect paths.

## Invariants

1. Exactly one normal composition root selects concrete implementations.
2. Orchestrator owns effect order; `latticed` and its adapters do not reorder,
   skip, or synthesize delivery stages.
3. Concrete adapters are constructed at the edge, implement typed ports, and
   never call one another.
4. Canonical `latticed` MCP enumeration contains exactly the seven approved
   names. Delivery and Runtime Status schemas remain zero-parameter; task
   schemas remain closed to the legacy canary or bounded create-only objective,
   one idempotency key, at most one catalog locator, and returned `task_ref`.
   Alternate `lattice-full-chain` is an observer only: it exposes the two
   delivery names, cannot dispatch Delivery Run, and never exposes or
   dispatches either task tool.
5. No MCP request can carry shell, SQL, path, credential, provider/execution
   settings, process, Git/test command, actor/session authority, lease, fence,
   or writable Codex-thread data. A bounded objective is inert task data only.
6. MCP calls reach the same `FullChainService` / Orchestrator composition as
   the normal `latticed` action. The MCP adapter is not a second gateway or
   orchestrator and cannot represent broader OpenClaw or protected Guardian
   actions.
7. `lattice-runtime delivery-run` cannot execute an official Codex writer or
   mint MCP/tunnel ingress evidence. Its scripted fixture is visibly fake and
   cannot substitute for canonical `latticed` acceptance.
   `lattice-full-chain` likewise cannot enter an official writer path; its
   retained Delivery Run name returns a fixed denial before service effects.
8. Startup, adapter, protocol, test, Git, database, or evidence ambiguity fails
   closed and never reports success.
9. Scripted protocol evidence remains visibly distinct from an official Codex
   app-server live turn.
10. Graphify executable, source repository, commit, staging root, fixed memory
    query, timeout, and database configuration are process-owned and digest-
    bound. They never enter MCP schemas or echo secrets.
11. The server-owned canary Task Spec digest remains that execution path's sole
    binding. A general task instead uses the Task-Ledger-owned submission
    envelope and formal Registry-bound stream identity; neither binding can
    substitute for the other or grant later execution authority.
12. Task lifecycle/intake, exact idempotency, audit,
    lease/fencing, and status are PostgreSQL-backed and fail closed when their
    independently verified current heads are unavailable.
13. Public task status is an allowlist of typed state/disposition and digests;
    it contains no raw spec, prompt, diff, command, path, secret, lease/fence,
    child output, or database detail.
14. Neither Task-Spec lifecycle nor general-intake edge wrapper can construct
    event/receipt fragments or treat a PostgreSQL row as authoritative before
    Task Ledger replay/current-head verification. The Task-Spec wrapper cannot
    bypass Task Domain validation; the intake wrapper cannot represent a Task
    Spec, transition, result, autonomy receipt, or effect at all.
15. Neither canary nor general intake can fabricate Project Registry authority.
    Control is only a locator and general creation requires independently
    current PostgreSQL Registry evidence. Policy/approval/writer composition
    remains mandatory before any project-selectable progression or effect.
16. Missing, active-at-completion, or physically corrupt Writer Lease history
    cannot be downgraded to a valid terminal Task status or recovery path.
17. A controlled-canary required-profile stream without its exact second autonomy event cannot
    transition, replay as completed, or produce normal Task Status. The
    one-event prefix maps only to reconciliation; historical optional streams
    remain byte-compatible.
18. Runtime health is a connection-only fact. It cannot be used as evidence
    that a delivery was started, completed, failed, reconciled, or corrupted.
    Receipt state is separately verified durable evidence and cannot imply that
    the database is currently reachable outside that read.
19. Historical status observation grants no successor ingress authority.
    Historical non-terminal and Completed streams remain current-profile-bound.
20. Normal no-argument MCP startup and every tool call are migration-free.
    Only explicit `--postgres-bootstrap` may iterate the exact Store/Memory/
    Writer profile closure: complete v5/v6 prerequisites, apply Writer v4 at
    exact v6 current, apply Store v7 with the fixed v4 rebind, apply Writer v5,
    perform its fixed Store-v8 runtime rebind, apply the Store-v8 successor,
    and install or rebind Foreman v1 to exact Store v8. It closes migrator
    credentials, constructs fresh runtime clients, verifies Foreman plus Task
    Ledger replay, and only then reports readiness.
21. Before changing admission, bootstrap consumes only PostgreSQL Memory 1.5
    and PostgreSQL Writer Lease 2.3 closed read-only profiles. It accepts only Store-v5 +
    Memory `Empty|V2|V3` + Writer `V5FallbackRequired`, Store-v5 + Memory `V3`
    + Writer `V5Bridge`, or Store-v6 + Memory `V3` + Writer
    `V6BridgePending|V6Current|V6V4Bridge|V6V4BridgeLegacyF252Rebind`, or
    Store-v7 + Memory `V3` + Writer
    `V7V4Current|V7V5RebindPending|V7V5Current`, or exact legacy/current
    Store-v8 + Memory `V3` + Writer `V8V5RebindPending|V8V5Current`. Every
    other triple fails closed. Only the v5
    fallback runs Store verification, Memory apply/verify, Writer-v2
    apply/verify, then exact Writer-v3 bridge. Composition never parses Memory
    or Writer rows or substitutes its own classifier.
    A physically fresh Store first requires the Writer-owned inspector to prove
    an exactly absent Writer namespace before Store v5 is created, after which
    the same closed triple is re-inspected. A partial Writer namespace fails
    before Store schema creation. Product bootstrap rejects Store legacy
    prefixes v1-v4 before admission observation or mutation; the exact
    nine-entry `V8LegacyPrefix` is the sole accepted successor predecessor.
22. Exact v6 bridge-pending may run only `V6Rebind`; exact v6 current may run
    only Writer-owned `V4Apply`; exact `V6V4BridgeLegacyF252Rebind` may run only
    the same Writer-owned `V4Apply`, which accepts that one quarantined
    predecessor, replaces only its fixed procedure, and reclassifies before
    composition continues. Exact `V6V4Bridge` may run only Store-owned
    `V7Apply`, whose transaction invokes the fixed v4 rebind. Exact
    Store-v7/Writer-v4 advances only through Writer-v5 apply; exact Writer-v5
    then advances only through its Store-v8 rebind and Store-owned v8 apply.
    Exact Store-v8 + Memory-v3 + `V8V5Current` is the sole verify-only terminal
    state. The coordinator may keep only its idle migrator session holding the
    outer gate while it installs/rebinds Foreman, restores configured admission,
    and closes migrator credentials. Fresh Runtime-role Task Ledger replay and
    Foreman verification then prove readiness; the Store verifier pins the
    Writer-v5 companion catalog, and normal deployed startup constructs the
    Writer repository before serving MCP.
23. Task-ingress idempotency is scoped by the process-owned ingress plus
    `client_request_id` and is shared across controlled-canary and general
    submission. Exact objective/formal-project retries return the retained
    `task_ref`; changed objective, project binding, or submission mode returns
    typed substitution and creates no second task, including after restart.
    Concurrent identical requests that observed different snapshots of the
    same effective project reload and return the one committed winner rather
    than surfacing a false changed-request conflict. A first Registry
    register/observe currentness collision permits one complete fresh
    Control/Git/Registry pass pinned to the initially selected project ID; it
    may reuse an identical active formal observation, but cannot reuse a stale
    physical observation or retarget the selector. If the newer-snapshot
    caller has not committed yet, the stale caller may re-resolve only that
    same effective project and retry admission once before its final winner
    reload; it must not poll, sleep, retarget, or retry without a bound.
24. No-selector general intake succeeds only when exactly one eligible Control
    project can be resolved. Exact ID/name selection must also yield one
    readable current project. Current directory, first row, legacy row, or
    conversational inference can never choose the project.
25. Objective/project-name validation occurs before durable mutation: non-empty,
    trimmed, already NFC, within fixed byte bounds, no NUL/control character,
    and no recognized secret material. Client request IDs and project IDs must
    also match their closed ASCII forms and be secret-free before project
    resolution; the retained formal project snapshot ID is secret-free before
    persistence or public projection. The objective is never executed or interpolated into shell,
    path, SQL, configuration, permission, or prompt authority.
26. General creation itself stops at replay-verified `DRAFT` after exactly one
    `GENERAL_TASK_INTAKE_V1` `TASK_CREATED` event. Only the separately composed
    managed-foreman successor may progress beyond that boundary, and only after
    replaying the immutable promotion plus current task/spec/budget-bound local
    execution authority.
27. General Status is reconstructed by `task_ref` from the PostgreSQL envelope,
    successor Task Ledger stream, subordinate foreman rows, Approval evidence,
    and Artifact Store bytes. It never trusts process memory or repeats
    Control/Registry mutation, Codex start, Writer acquisition, Git mutation,
    or another downstream effect.
28. A managed worker is real-running only after the exact retained
    `turn/started` for its thread/turn and before an exact terminal. Restart
    reads, resumes, and reconciles those retained IDs before any replacement
    attempt. The default limits are four active attempts globally, one per
    task, and two repair retries.
29. Managed verification executes only fixed argument vectors selected by the
    captured Task Spec/trusted repository policy. A worker message, exit zero,
    or Git object is not completion; successful programming work stops at
    `AWAITING_MERGE_APPROVAL`, and local execution authority cannot authorize
    push, merge, deployment, publication, payment, external messaging, or
    permanent deletion.
30. A generation and checkpoint digest do not independently authorize managed
    dispatch. Initial and restart identity loads must replay one uniquely
    active, continuing `SoleForemanBinding`; empty, blocked, completed,
    multi-active, or wrong-next-action projections fail before task claim.

## Allowed Dependencies

- `lattice-contracts` 1.15, `lattice-ports` 2.4,
  `orchestrator-runtime` 3.0, Foreman State 1.5, Task Ledger 3.0, PostgreSQL
  Store 1.23, PostgreSQL Memory 1.5, Writer Lease domain 1.1, PostgreSQL Writer Lease 2.2,
  PostgreSQL Foreman 1.0, Approval Verifier 1.1, and Artifact Store 1.1 public
  APIs.
- Concrete Codex, PostgreSQL Task Ledger, bounded workspace/Git, and fixed-test
  adapters required by TASK-032, only for construction and port
  implementation.
- Concrete exact-snapshot, Graphify 1.0, pure Codebase Memory 1.0, and
  PostgreSQL Memory 1.5 adapters required by TASK-033/TASK-105, only for
  construction and typed bootstrap inspection/port implementation.
- Bounded stdio/JSON/MCP framing, process configuration, hashing, timeout, and
  diagnostics libraries required at the application edge.
- Concrete PostgreSQL Task Ledger and PostgreSQL Writer Lease 2.1 adapters only
  for construction behind their typed boundaries.
- `lattice-hermes-adapter` 1.1 only for production runner construction,
  ephemeral liveness, and lifecycle teardown; it grants no durable truth,
  credential ownership, or orchestration authority.

## Forbidden Dependencies

- Orchestrator internals, direct policy/domain mutation, OpenClaw SDK as a
  second gateway, Graphify/Hermes/Memory mutation, Guardian trust roots,
  deployment/publication/payment code, or companion/playmate website code.
- Arbitrary shell/SQL execution, caller-selected filesystem/product paths,
  credential payloads, dynamic provider/tool registration, or an alternate
  durable ledger.
- Adapter-to-adapter calls or reverse dependencies from an adapter into the
  composition root.

## Failure, Compatibility, And Migration

Unknown startup configuration, extra MCP arguments, duplicate/unknown tools,
adapter construction failure, malformed framing, interrupted output, or an
uncertain downstream effect returns a bounded typed failure or reconciliation
result. No permissive fallback or generic command surface is allowed.

Version 1.0 is implemented in the existing `apps/lattice-runtime` package. The
`latticed` executable is the canonical normal entry; the existing
`lattice-runtime` name remains a compatibility wrapper over the same
composition. Removing or behaviorally diverging that wrapper requires a
versioned migration decision and compatibility evidence.

Version 1.2 adds two bounded task tools and one fixed server-derived MCP actor.
It does not add a second listener, per-human identity claim, generic task
surface, alternate task store, or direct writer control. Clients that do not
refresh discovery retain the two existing delivery contracts.

Version 1.3 closes the legacy `lattice-runtime delivery-run` official lane.
The command remains only for the exact repository-owned scripted fixture and
fails closed for `OFFICIAL_CODEX_APP_SERVER`. This prevents caller-selected
CLI paths from becoming a second official workspace/adapter entry or being
misrecorded as an MCP tunnel actor. The four canonical `latticed` tools and
their schemas are unchanged.

Version 1.5 adds an exact standalone Hermes process-lifecycle flag to canonical
`latticed`. It changes neither the four-tool MCP contract nor durable task
truth, provider credentials, dependency direction, or orchestration order.

Version 1.9 adds non-MCP, read-only PostgreSQL health and receipt-state probes
to the compatibility CLI. They deliberately separate durable-facts availability
from delivery-receipt interpretation; the canonical four MCP tools and their
schemas are unchanged.

Version 2.0 establishes the four-core Runtime operating boundary. PostgreSQL
is the only durable source of facts and receipts. Graphify is derived
relationship memory that may be rebuilt from PostgreSQL, and Hermes is a
read-only reflective advisor whose findings remain inferences rather than
facts. A Graphify or Hermes failure therefore degrades only that capability;
it must not block control or durable-fact reads. PostgreSQL unavailability
does make the Runtime unavailable for fact-dependent work. Existing historical
full-chain entry points remain compatibility/explicit-integration paths, not
the normal acceptance requirement for each component.

Version 2.2 makes the Runtime a single local composition with independently
verifiable internal departments: `CORE_ONLY`, `GRAPHIFY`, and
`GRAPHIFY_HERMES`. Core projections use the durable delivery receipt digest;
Graphify and Hermes add derived evidence only and degrade visibly without
invalidating core truth.

Version 2.4 adds a local, process-configured Graphify refresh for a clean
immutable Git source. It keeps the five-tool MCP surface unchanged and does
not treat a derived-memory receipt as delivery evidence.

Version 2.6 separates historical non-success terminal observation from current
ingress mutation authority. It adds no event, migration, tool, field, or
external mutation or execution and preserves schema v5, Memory v3, and Writer
Lease v2. Its status proof still performs the bounded database and loopback
reads required to verify durable truth.

Version 1.8 emits the Task Ledger 2.3 required-profile marker for new
controlled-task admissions and fails closed when required receipt replay is
incomplete. It keeps exactly four MCP tools and the existing six-field task
status output, and requires fresh canonical `latticed` restart evidence rather
than a Store test binary as acceptance.

Version 3.4 corrects the unreleased general intake path to use the separate
`GENERAL_TASK_INTAKE` identity and `TaskIntakeLifecyclePort`. It persists only
the shared ingress claim, envelope, and one `GENERAL_TASK_INTAKE_V1`
`TASK_CREATED` event, with no Task Spec, currency, autonomy, progression,
Writer Lease, or effect. Control remains only a locator; PostgreSQL Project
Registry and Task Ledger remain authority; canary v2 remains compatible. The
explicit bootstrap reaches the required schema-v8 successor only through the
append-only Writer-v4/v5 and Foreman rebind sequence and finishes with
fresh-runtime verification.

Version 3.5 composes the optional process-owned managed-foreman lane after that
unchanged intake boundary. It uses one immutable Task-Spec successor, the
existing Task lifecycle and Writer Lease, formal sole-foreman checkpoint,
exact Codex connector, Approval Verifier, Artifact Store, and subordinate
same-database PostgreSQL foreman extension. Dispatch is asynchronous and
bounded; restart reconciles retained exact IDs; status v4 is replay-only; a
verified programming result stops before every protected external effect.

Version 3.6 makes the replayed sole-foreman lifecycle state an explicit
preclaim authority gate. Formal identity construction now requires one active,
continuing latest generation and rejects empty, terminal, or ambiguous runtime
projections before task claim or provider dispatch.

Version 3.7 makes durable intake discovery the supervisor-owned preparation
entry, pins one clean immutable promotion intent before successor effects,
projects unpromoted/deferred work as read-only v4 status, retains a bounded
rebuttable preparation observation, and holds the complete verified effect
bundle plus supervised Git process tree for the process lifetime. It does not
add an MCP authority or protected-action permission.

## Acceptance Gates

| Gate | Evidence | Owner | Required for merge |
|---|---|---|---|
| Composition direction | Cargo metadata proves orchestrator has no concrete dependency and only `latticed` selects adapters | Architecture review | yes |
| MCP tool closure | exact seven-tool list; unchanged zero-argument Delivery/Runtime Status requests; versioned Runtime dependency projection; closed task/checkpoint schemas; legacy exact two with checkpoint unknown; unknown/additional-property rejection | Engineering | yes |
| Restricted input | objective/selector bounds plus shell/SQL/path/credential/provider/actor/session/lease/fence/writable-thread rejection before durable mutation | Security review | yes |
| One Gateway | both task tools plus Foreman Checkpoint invoke the same `FullChainService` / Orchestrator composition; MCP has no direct database/Codex/Git/Writer call path | Architecture review | yes |
| Fixed identity | process profile supplies the actor/audit binding; tunnel/local commitments cannot substitute; hostile `clientInfo`/arguments grant no authority | Security review | yes |
| Durable task control | Task creation/idempotency/audit/status replay from PostgreSQL with fresh-process equality | Engineering | yes |
| Registered general intake | unique Control locator, live double-read/physical observation, exact Registry authority/currentness, no arbitrary path/default guess, shared-key exact retry/substitution, distinct no-spec/no-currency subject, one create event, v3 `DRAFT` restart replay, and zero autonomy/execution/model/writer effects | Engineering and security review | yes |
| Managed general foreman | exactly-once promotion, current local authority, atomic capacity claim, exact start, bounded retry/reconcile, independent fixed-command verification, v4 replay, real Codex disposable happy path, and restart without duplicate Agent | Engineering, architecture, and security review | yes |
| Store-v8 bootstrap closure | exact accepted cross-product, legacy-v8 compatibility, v6/v7 Writer successors, Store-v8 apply, Foreman rebind, failure-stopped retry, concurrent idempotency, and fresh runtime-role replay | Architecture and integration review | yes |
| Controlled-canary autonomy profile | required canary marker, exact second receipt, historical optional replay, pending reconciliation, and fresh-`latticed` Status with no extra wire field | Engineering and security review | yes |
| Writer authority | real PostgreSQL lease/fencing/current-head evidence; no fake/synthetic production path | Security review | yes |
| Legacy command isolation | `lattice-runtime delivery-run` accepts only the exact scripted fixture; official Codex and MCP/tunnel provenance fail before effects | Compatibility review | yes |
| Delivery acceptance | official Codex turn, isolated scope/test/commit, durable outcome and separate restart/status replay | Engineering | yes |
| Failure closure | startup/framing/adapter/timeout/unknown-effect cases never report success | Engineering | yes |
| Historical terminal status | cross-binary Failed replay, tamper rejection, exact task-ref check, and zero durable mutation | Engineering and security review | yes |
| Standalone Hermes lifecycle | exact CLI routing, runner liveness, explicit bounded teardown, redacted errors, and local live-start evidence | Engineering and security review | yes |
| Hermes executed-input closure | exact twelve-setting preflight, ignored legacy-helper sentinels, v2 adapter receipt and direct launcher plan | Engineering and security review | yes |

## Change Policy

Composition ownership, tool names or schemas, compatibility-entry behavior,
adapter dependency direction, credential boundary, gateway separation, or
success/failure semantics require a versioned constitution amendment,
SPEC/ADR trace, architecture review, and responsible-user approval. This
constitution cannot be weakened merely to excuse implementation drift.

## Amendment History

| Version | Date | Decision reference | Summary | Approver |
|---|---|---|---|---|
| 3.9 | 2026-08-31 | ADR-029 managed-foreman deployment repair | Serialize the exact Store-v8 runtime successor, Writer-v5 and Foreman-v1 rebinds, stopped failure retry, and fresh-runtime verification without widening MCP startup | User-authorized deployment repair |
| 3.7 | 2026-08-27 | SPEC-011 durable-core review, ADR-028 | Add supervisor-owned durable intake preparation, immutable pre-successor source intent, unpromoted v4 blocker replay, and process-lifetime effect/Git containment | Delegated product owner |
| 3.6 | 2026-08-27 | SPEC-011 v1.2, ADR-028 | Require replay-verified unique ACTIVE and CONTINUE Foreman Runtime state before formal managed identity, task claim, or provider dispatch | Delegated product owner |
| 3.1 | 2026-08-25 | SPEC-009, ADR-027, TASK-105 live correction | Add the Memory 1.3 and PostgreSQL Writer Lease 1.8 typed read-only bootstrap profiles as the sole pre-admission cross-module contract, including exact empty, predecessor, bridge-pending, current, fresh-absence, and legacy-product-rejection handling | Sole-foreman delegation |
| 3.2 | 2026-08-25 | SPEC-010, TASK-106 | Accept one closed dependency blocker, prove owned Git binding before block/resume, and expose restart-restored dependency next action without adding a tool or database schema | Explicit user delegation |
| 3.3 | 2026-08-26 | ADR-023 Phase 3 amendment | Initial registered-project general Task Submit; superseded before release by 3.4 because intake must not reuse Task-Spec/autonomy lifecycle semantics | User-authorized Phase 3 |
| 3.4 | 2026-08-26 | ADR-023 Phase 3 P1 correction | Use a distinct pre-specification intake identity and admit/load-only port with one create event, no currency/autonomy/progression, append-only Writer-v4/Store-v7 bootstrap closure, and unchanged canary/protected-action gates | User-authorized Phase 3 |
| 3.5 | 2026-08-26 | SPEC-011, ADR-028 | Compose the formal bounded managed foreman successor, exact Codex lifecycle, PostgreSQL evidence replay, independent verification, v4 status, and protected-effect separation without weakening create-only intake | Delegated product owner |
| 3.8 | 2026-08-30 | ADR-023 deployment compatibility amendment | Bind the managed runtime to Store 1.23, Memory 1.5, and the combined Writer compatibility profile; preserve duplicate pre-v7 histories without a chosen winner and fail closed through the shared ingress boundary | User-authorized deployment hotfix |
| 3.0 | 2026-08-25 | SPEC-009, ADR-027, TASK-105 live correction | Consume Writer 1.8's closed preflight before admission effects; enforce the exact Store/Writer cross-product and make v6-current a full fresh-runtime verify-only path with zero durable mutation | Sole-foreman delegation |
| 2.9 | 2026-08-25 | SPEC-009, ADR-027, TASK-105 live correction | Make schema-v5 bootstrap strictly Writer-first; an existing exact v3 bridge skips generic Memory verification, while only unsupported foundation enters the complete Store, Memory, Writer-v2, then Writer-v3 fallback | Sole-foreman delegation |
| 2.8 | 2026-08-25 | SPEC-009, ADR-027, TASK-105 live correction | Let the Writer-owned v3 boundary recognize an exact existing schema-v5 bridge before the generic Store-v5/Writer-v2 fallback; only exact unsupported foundation may fall back and every other Writer error remains fail-closed | Sole-foreman delegation |
| 2.7 | 2026-08-25 | SPEC-009, ADR-027, TASK-105 | Add the seventh canonical foreman checkpoint, verified zero-argument status replay and explicit Writer-v3-before-Store-v6 bootstrap; legacy surface unchanged | Sole-foreman delegation |
| 1.0 | 2026-08-05 | SPEC-002 v25, ADR-021, TASK-032 | Sole normal composition root in `apps/lattice-runtime`, two fixed zero-parameter MCP tools, and one shared `lattice-runtime` compatibility wrapper | User approval in preceding implementation window |
| 1.1 | 2026-08-05 | SPEC-002 v26, ADR-022, TASK-033 | Compose the exact Graphify/PostgreSQL Codebase Memory continuation while preserving the same two-tool MCP boundary | User TASK-033 direction |
| 1.2 | 2026-08-09 | SPEC-003 v3, ADR-023, TASK-038 | Add bounded Task Submit/Status through the same Gateway with a fixed server-owned actor, Task Spec digest unity, PostgreSQL task truth, and real writer lease/fencing | User TASK-038-first direction |
| 1.3 | 2026-08-10 | SPEC-003 v3, ADR-023 security correction, TASK-038 | Make canonical `latticed` the sole official writer entry and restrict the legacy CLI delivery command to the exact visibly scripted fixture | User TASK-038-first security boundary |
| 1.4 | 2026-08-10 | SPEC-003 v4, ADR-023 alternate-entry correction, TASK-038 | Restrict alternate `lattice-full-chain` to a read-only delivery observer and reserve all official mutation plus task control for canonical `latticed` | User TASK-038-first One Writer boundary |
| 1.5 | 2026-08-13 | SPEC-002 v29, ADR-021 clarification, TASK-060 | Add canonical `latticed --hermes-launch` as bounded owner of the existing production Hermes runner without changing MCP, truth, credential, or dependency boundaries | User goal-mode direction to complete Hermes |
| 1.6 | 2026-08-13 | SPEC-002 v30, ADR-021 clarification, TASK-064 | Add opt-in lazy production Hermes composition to canonical four-tool `latticed`; preserve task/status zero-effect paths and require explicit teardown | User goal-mode direction to integrate Hermes into LATTICE |
| 1.7 | 2026-08-13 | SPEC-002 v31, TASK-065 | Remove two non-executed broker-helper settings from production admission while preserving the direct verified Codex proxy, lazy activation, and MCP contracts | User goal-mode direction to complete Hermes |
| 1.8 | 2026-08-15 | SPEC-002 v35, ADR-011/019, TASK-050 | Emit and enforce the required Task-created autonomy profile through Task Ledger 2.3 while preserving the four-tool/six-field MCP wire | User-approved TASK-050 repair amendment |
| 1.9 | 2026-08-22 | Runtime direction | Add read-only `runtime-health` and `receipt-state` so PostgreSQL connectivity cannot be confused with delivery receipt state | User-approved four-part Runtime direction |
| 2.0 | 2026-08-22 | Runtime direction | Operate LATTICE, PostgreSQL, Graphify, and Hermes as one Runtime with independent verification and defined degraded modes; reserve full-chain execution for explicit integration | User-approved four-part Runtime direction |
| 2.1 | 2026-08-22 | Runtime direction | Make core-only MCP operation the executable default and reserve Graphify/Hermes continuation for explicit full-chain integration | User-approved four-part Runtime direction |
| 2.2 | 2026-08-22 | Runtime direction | Split optional Runtime composition into Graphify and Graphify-plus-Hermes modes; optional failure preserves PostgreSQL truth and reports degradation | User-approved four-part Runtime direction |
| 2.3 | 2026-08-22 | Runtime direction | Add a read-only Runtime Status MCP tool so Codex can independently verify PostgreSQL, Graphify, and Hermes state without a full-chain run | User-approved four-part Runtime direction |
| 2.4 | 2026-08-22 | Runtime direction | Add a process-configured exact-Git Graphify refresh without a new MCP tool or delivery dependency | User-approved independent-core direction |
| 2.5 | 2026-08-23 | Evidence-driven autonomy direction | Add one zero-parameter, read-only delivery-reconciliation probe. It can report a replayed durable fact or a blocker, but cannot run Codex, mutate evidence, or turn uncertainty into success. | Current user task |
| 2.6 | 2026-08-24 | ADR-025, SPEC-008 v2 | Permit verified historical non-success terminal status after binary drift without granting mutation authority | User-authorized bounded repair |
