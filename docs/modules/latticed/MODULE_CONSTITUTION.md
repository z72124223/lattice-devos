---
module_id: latticed
name: LATTICE Normal Composition Root
version: 2.3
status: active
owner: LATTICE maintainers
last_reviewed: 2026-08-22
---

## Mission

Provide the sole normal local composition root that constructs the pure Rust
orchestrator with concrete delivery/task adapters and exposes the bounded Codex
App MCP stdio surface. The existing `apps/lattice-runtime` package implements
this module and maps every MCP operation into the same `FullChainService` /
Orchestrator composition.

## Non-Goals

- Own domain transitions, policy, workflow order, durable task truth, provider
  semantics, or product-code mutation logic.
- Become a second gateway service, database authority, product writer,
  approval surface, Guardian, release controller, or deployment service.
- Accept arbitrary task text, shell, SQL, path, credential, provider, process,
  Git, test-command, actor/session, lease, fence, or writable-thread input
  through MCP.
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

- Construct one Orchestrator 2.6 instance with typed Contracts 1.13 / Ports 1.9
  implementations for the bounded delivery, graph-memory, and task-control
  paths.
- Through canonical `latticed`, expose exactly five MCP tools:
  `lattice_delivery_run`, `lattice_delivery_status`, `lattice_runtime_status`,
  `lattice_task_submit`, and `lattice_task_status`. Runtime Status is read-only,
  starts no optional component, and reports their independent readiness/degradation.
- Through the compatibility `lattice-runtime` executable, expose one non-MCP,
  read-only `runtime-health` and `receipt-state` commands. They accept the same
  fixed marker-owned PostgreSQL binding as Delivery Status. `runtime-health`
  reports control-core readiness, PostgreSQL connection availability, and the
  configured independent Graphify/Hermes mode; it always keeps
  `delivery_receipt=NOT_INSPECTED`. `receipt-state` reports only the verified
  durable receipt projection. Neither command creates or reinterprets a
  receipt, and neither alters the five-tool MCP surface.
- Restrict the alternate `lattice-full-chain` executable to a legacy observer
  surface. It advertises only the two delivery names, rejects Delivery Run
  with a fixed code before service dispatch, permits only durable Delivery
  Status reads, and treats both task names as unknown under legacy and
  stateless MCP.
- Preserve zero-parameter closed schemas for both delivery tools. Give Task
  Submit only the closed `CONTROLLED_CODEX_CANARY` intent plus one bounded
  `client_request_id`, and give Task Status only the lowercase SHA-256
  `task_ref` returned by Submit. Every schema has
  `additionalProperties: false`.
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
- Select every project, snapshot, repository/base, scope, verification,
  capability, budget, approval, prompt, workspace, and downstream binding from
  process-start composition configuration, never from MCP arguments.
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
- Construct the complete Task Spec 2.1 from the server-owned canary template,
  revalidate it through Task Domain, and preserve its one digest across
  Gateway, Task Ledger, Writer Lease, Codex, verification/Git, and status
  evidence.
- Compose the injected Writer Lease 1.1 repository and Task lifecycle
  port. Production task dispatch cannot use `FakeWriterLease`, synthetic
  authority, or a process-memory task/status store.
- The local Task lifecycle edge wrapper implements `TaskLifecyclePort` only by
  translating typed admit/transition/result/load calls into Task Ledger 2.3 and
  `PostgresTaskLedger` public append/replay APIs. New admissions carry the
  exact required-profile marker, and canonical autonomy receipt construction/
  verification remains Task-Ledger-owned. The wrapper owns no transition
  legality, receipt semantics, SQL/schema, alternate cache, or workflow order.
- After the preconfigured scripted delivery receipt, the run tool invokes the
  same coordinator's exact-snapshot Graphify/memory node. The status tool loads
  delivery plus exact analysis/retrieval evidence from PostgreSQL; neither tool
  gains an argument, and task tools cannot alter their delivery binding.
- Task Submit/Status remains `WriterOnly`: after the durable result and Writer
  Lease release it returns status without running Graphify, Hermes, or Memory.
  The fixed canary accepts only a process deadline above the 30-second cleanup
  reserve and at most 300 seconds, below its 600-second lease TTL. Longer task
  profiles remain forbidden until heartbeat, interruption, and orphan recovery
  are composed.
- Retain `lattice-runtime delivery-run` only as a visibly scripted, exact
  repository-owned acceptance fixture. It rejects official Codex mode before
  identity, database, workspace, or process effects; canonical `latticed` is
  the only official writer entry.
- Keep OpenClaw as the broader normal human gateway. MCP permits only the one
  fixed typed Submit template and its Status projection; it provides no
  general submit/plan/approve/reject/stop or protected-release operation.
- Return only the allowlisted typed status projection and reconstruct it from
  PostgreSQL in a fresh process/session without rerunning external effects.
  A Completed projection additionally requires an existing, independently
  replayed Writer Lease project with no current authority and the fixed canary
  `1/2/2` fence/transition/command history. `Merging + result` recovery verifies
  active `1/1/1` or released `1/2/2` before any further Task Ledger mutation.

## Invariants

1. Exactly one normal composition root selects concrete implementations.
2. Orchestrator owns effect order; `latticed` and its adapters do not reorder,
   skip, or synthesize delivery stages.
3. Concrete adapters are constructed at the edge, implement typed ports, and
   never call one another.
4. Canonical `latticed` MCP enumeration contains exactly the five approved
   names. Delivery and Runtime Status schemas remain zero-parameter; task schemas remain closed
   to the one approved intent/`client_request_id` and returned `task_ref`.
   Alternate `lattice-full-chain` is an observer only: it exposes the two
   delivery names, cannot dispatch Delivery Run, and never exposes or
   dispatches either task tool.
5. No MCP request can carry shell, SQL, path, credential, provider, arbitrary
   task text, process, Git/test command, actor/session authority, lease, fence,
   or writable Codex-thread data.
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
11. The server-owned Task Spec digest is the sole task binding; legacy fixed
    submission, profile/run, or synthetic authority digests cannot substitute.
12. Task lifecycle, exact idempotency, audit,
    lease/fencing, and status are PostgreSQL-backed and fail closed when their
    independently verified current heads are unavailable.
13. Public task status is an allowlist of typed state/disposition and digests;
    it contains no raw spec, prompt, diff, command, path, secret, lease/fence,
    child output, or database detail.
14. The Task lifecycle edge wrapper cannot construct event/receipt fragments,
    bypass Task Domain validation, or treat a PostgreSQL row as authoritative
    before Task Ledger replay/current-head verification.
15. The fixed canary cannot fabricate a live Project Registry authority or be
    generalized into project-selectable/free-form work before durable
    Registry currentness and normal Policy composition exist.
16. Missing, active-at-completion, or physically corrupt Writer Lease history
    cannot be downgraded to a valid terminal Task status or recovery path.
17. A required-profile stream without its exact second autonomy event cannot
    transition, replay as completed, or produce normal Task Status. The
    one-event prefix maps only to reconciliation; historical optional streams
    remain byte-compatible.
18. Runtime health is a connection-only fact. It cannot be used as evidence
    that a delivery was started, completed, failed, reconciled, or corrupted.
    Receipt state is separately verified durable evidence and cannot imply that
    the database is currently reachable outside that read.

## Allowed Dependencies

- `lattice-contracts` 1.13, `lattice-ports` 1.9,
  `orchestrator-runtime` 2.6, Task Ledger 2.3, and Writer Lease 1.1 public APIs.
- Concrete Codex, PostgreSQL Task Ledger, bounded workspace/Git, and fixed-test
  adapters required by TASK-032, only for construction and port
  implementation.
- Concrete exact-snapshot, Graphify 1.0, and pure Codebase Memory 1.0 adapters
  required by TASK-033, only for construction and port implementation. The
  proposed independent PostgreSQL Memory extension adapter cannot be composed
  until its owning module receives an explicitly approved versioned amendment.
- Bounded stdio/JSON/MCP framing, process configuration, hashing, timeout, and
  diagnostics libraries required at the application edge.
- Concrete PostgreSQL Task Ledger and PostgreSQL Writer Lease 1.0 adapters only
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

Version 1.8 emits the Task Ledger 2.3 required-profile marker for new
controlled-task admissions and fails closed when required receipt replay is
incomplete. It keeps exactly four MCP tools and the existing six-field task
status output, and requires fresh canonical `latticed` restart evidence rather
than a Store test binary as acceptance.

## Acceptance Gates

| Gate | Evidence | Owner | Required for merge |
|---|---|---|---|
| Composition direction | Cargo metadata proves orchestrator has no concrete dependency and only `latticed` selects adapters | Architecture review | yes |
| MCP tool closure | exact five-tool list; unchanged empty delivery/Runtime Status schemas; closed task schemas and unknown/additional-property rejection | Engineering | yes |
| Restricted input | shell/SQL/path/credential/provider/task-text/actor/session/lease/fence/writable-thread matrix is rejected before dispatch | Security review | yes |
| One Gateway | both task tools invoke the same `FullChainService` / Orchestrator composition; MCP has no direct database/Codex/Git call path | Architecture review | yes |
| Fixed identity | process profile supplies the actor/audit binding; tunnel/local commitments cannot substitute; hostile `clientInfo`/arguments grant no authority | Security review | yes |
| Durable task control | Task creation/idempotency/audit/status replay from PostgreSQL with fresh-process equality | Engineering | yes |
| Required autonomy profile | new marker, exact second receipt, historical optional replay, pending reconciliation, and fresh-`latticed` Status with no extra wire field | Engineering and security review | yes |
| Writer authority | real PostgreSQL lease/fencing/current-head evidence; no fake/synthetic production path | Security review | yes |
| Legacy command isolation | `lattice-runtime delivery-run` accepts only the exact scripted fixture; official Codex and MCP/tunnel provenance fail before effects | Compatibility review | yes |
| Delivery acceptance | official Codex turn, isolated scope/test/commit, durable outcome and separate restart/status replay | Engineering | yes |
| Failure closure | startup/framing/adapter/timeout/unknown-effect cases never report success | Engineering | yes |
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
