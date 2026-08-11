---
module_id: latticed
name: LATTICE Normal Composition Root
version: 1.9
status: active
owner: LATTICE maintainers
last_reviewed: 2026-08-11
---

## Mission

Provide the sole normal local composition root that constructs the pure Rust
orchestrator with concrete delivery/task adapters and exposes the bounded MCP
surface. The existing `apps/lattice-runtime` package implements this module and
maps every MCP operation into the same `FullChainService` / Orchestrator
composition.

The current highest-priority product gate is a real ChatGPT <-> LATTICE
round-trip through this composition. `latticed` is the microkernel edge: it may
bind replaceable providers behind typed Ports, but it never transfers Task
truth, policy, writer authority, lease/fencing, or orchestration ownership to
MCP or another provider process.

## Non-Goals

- Own domain transitions, policy, workflow order, durable task truth, provider
  semantics, or product-code mutation logic.
- Become a second gateway service, database authority, product writer,
  approval surface, Guardian, release controller, deployment service, provider
  registry, or provider scheduler.
- Accept arbitrary task text, shell, SQL, path, credential, provider, process,
  Git, test-command, actor/session, lease, fence, or writable-thread input
  through MCP.
- Promote the legacy `lattice-runtime` scripted acceptance command into an
  official writer entry or let it claim MCP/tunnel provenance.
- Claim that local MCP tests, tunnel readiness, or tool discovery prove a real
  ChatGPT invocation or durable cross-session replay.
- Compose GH-9 Reflection queue/history/candidate Ports into production,
  execute Hermes, or start unattended Reflection work under this version.
- Convert every kernel module into an MCP server.

## Owned Data

- Process-lifetime composition configuration, constructed adapter instances,
  the bounded MCP stdio session, legacy fixture routing, and the verified fixed
  tunnel/profile ingress identity used to derive the one MCP actor.
- Connector/tool snapshot compatibility evidence for the exact published
  ChatGPT app revision, without treating that snapshot as task truth.
- No Task Ledger, project, workspace, Git, test, Codex-thread, approval,
  credential, memory, Reflection, provider-session, or release truth.
  PostgreSQL and their domain owners retain durable and semantic authority.

## Public Contracts

- Construct one Orchestrator 2.3 instance with typed Contracts 1.12 / Ports 1.9
  implementations for the bounded delivery, graph-memory, and task-control
  paths. Ports 1.9 Reflection traits remain uncomposed in production under this
  constitution version.
- Through canonical `latticed`, expose exactly four MCP tools:
  `lattice_delivery_run`, `lattice_delivery_status`,
  `lattice_task_submit`, and `lattice_task_status`.
- Restrict the alternate `lattice-full-chain` executable to a legacy observer
  surface. It advertises only the two delivery names, rejects Delivery Run with
  a fixed code before service dispatch, permits only durable Delivery Status
  reads, and treats both task names as unknown under legacy and stateless MCP.
- Preserve zero-parameter closed schemas for both delivery tools. Give Task
  Submit only the closed `CONTROLLED_CODEX_CANARY` intent plus one bounded
  `client_request_id`, and give Task Status only the lowercase SHA-256
  `task_ref` returned by Submit. Every schema has
  `additionalProperties: false`.
- Select every project, snapshot, repository/base, scope, verification,
  capability, budget, approval, prompt, workspace, and downstream binding from
  process-start composition configuration, never from MCP arguments.
  Credentials remain process-private input to their expressly approved trusted
  runtime boundary and never enter tool schemas, diagnostics, or results.
- For the fixed private P0 profile, the managed tunnel launch process, pinned
  official tunnel-client, and digest-pinned canonical `latticed` are one
  responsible-user-approved process-private credential boundary. Direct
  canonical `latticed` launch is required and `CONTROL_PLANE_API_KEY` must be
  available to both trusted runtime processes as inherited process input.
  This bearer grants control-plane authentication only; it is not Task input,
  actor, Policy, lease, writer, database, Guardian, approval, or release
  authority.
- Canonical `latticed` terminates that inheritance at every downstream spawn
  boundary. The supervised Codex child, verification/test and Git helpers,
  PostgreSQL tools, watchdogs, workspaces, Git content or metadata, logs,
  audit/database rows, results, and retained evidence must not receive or
  contain the bearer.
- The responsible user owns the retained private P0 Runtime key and requires
  its OpenAI Platform permission to remain `All`. Automation and agents must
  preserve that setting and may not restrict, rotate, recreate, delete, or
  otherwise change the key without a later explicit instruction from that
  user. The plaintext and full tracking identifier remain absent from tracked
  files and retained evidence.
- Map `lattice_delivery_run` to the one typed delivery coordinator and map
  `lattice_delivery_status` to the same durable ledger/status projection.
- Map Task Submit/Status into the same `FullChainService` and injected
  Orchestrator used by `latticed`; do not create an MCP-specific gateway,
  queue, cache, or workflow owner.
- Derive a closed ingress actor from verified process-start configuration;
  production Secure MCP Tunnel and local canonical acceptance use distinct
  non-substitutable actor/adapter commitments. `clientInfo` and caller identity
  fields grant no authority.
- Construct the complete Task Spec 2.1 from the server-owned canary template,
  revalidate it through Task Domain, and preserve its one digest across
  Gateway, Task Ledger, Writer Lease, Codex, verification/Git, and status
  evidence.
- Compose the injected Writer Lease 1.1 repository and Task lifecycle port.
  Production task dispatch cannot use `FakeWriterLease`, synthetic authority,
  or a process-memory task/status store.
- The local Task lifecycle edge wrapper implements `TaskLifecyclePort` only by
  translating typed admit/transition/result/load calls into the existing
  `PostgresTaskLedger` public append/replay API. It owns no transition legality,
  Task Ledger semantics, SQL/schema, alternate cache, or workflow order.
- After the preconfigured scripted delivery receipt, the run tool invokes the
  same coordinator's exact-snapshot Graphify/memory node. The status tool loads
  delivery plus exact analysis/retrieval evidence from PostgreSQL; neither tool
  gains an argument, and task tools cannot alter their delivery binding.
- Task Submit/Status remains `WriterOnly`: after the durable result and Writer
  Lease release it returns status without running Graphify, Hermes, Memory, or
  Reflection. The fixed canary accepts only a process deadline above the
  30-second cleanup reserve and at most 300 seconds, below its 600-second lease
  TTL. Longer task profiles remain forbidden until heartbeat, interruption,
  and orphan recovery are composed.
- Retain `lattice-runtime delivery-run` only as a visibly scripted, exact
  repository-owned acceptance fixture. It rejects official Codex mode before
  identity, database, workspace, or process effects; canonical `latticed` is
  the only official writer entry.
- Keep OpenClaw as the current broader human gateway and fallback while ChatGPT
  MCP undergoes P0 real-round-trip validation. ChatGPT becomes the primary
  conversational gateway only after the segmented acceptance gates pass and a
  later explicit provider-state decision promotes it.
- Return only the allowlisted typed status projection and reconstruct it from
  PostgreSQL in a fresh process/session without rerunning external effects.
  A Completed projection additionally requires an existing, independently
  replayed Writer Lease project with no current authority and the fixed canary
  `1/2/2` fence/transition/command history. `Merging + result` recovery verifies
  active `1/1/1` or released `1/2/2` before any further Task Ledger mutation.
- Bind any future external Codex/Hermes/Graphify/model MCP provider only behind
  its versioned provider-neutral Port. The MCP process cannot call another
  adapter directly or obtain kernel authority.

## ChatGPT Connectivity P0 Gates

The following gates are independent and must be reported separately:

### `DISCOVERY_VERIFIED`

A current real ChatGPT app/session discovers the exact four approved tools and
closed schemas for the exact connector snapshot. A historical two-tool snapshot
or local `tools/list` is not sufficient.

### `READ_ROUNDTRIP_VERIFIED`

A real ChatGPT request invokes a read-only LATTICE tool, reaches canonical
`latticed`, and returns a typed LATTICE response. Tunnel health, readiness, or
tool listing alone is insufficient.

### `BOUNDED_SUBMIT_VERIFIED`

A real ChatGPT request invokes only `CONTROLLED_CODEX_CANARY` through
`lattice_task_submit`; all prohibited properties remain rejected before
Gateway dispatch. If the ChatGPT workspace/plan/platform disallows the write
action, record `PLATFORM_CAPABILITY_BLOCKED` rather than changing Rust or
weakening schemas.

### `CROSS_SESSION_DURABLE_READ_VERIFIED`

A separate new ChatGPT request or session reads the same terminal Task result
through `lattice_task_status`, reconstructed from PostgreSQL, with zero repeated
Codex, verification, Git, Graphify, Hermes, Memory, or Reflection effects.

### `CONTROL_VERIFIED`

Pause, resume, cancel, approve, or reject operations are not part of version
1.5. They require new typed contracts, Policy decisions, threat analysis, and a
versioned amendment.

## Acceptance-First Engineering Rule

For the current highest P0, engineering order is value-first and evidence-
driven:

1. First prove the smallest real controlled GPT <-> LATTICE <-> Codex path in
   a fixed, disposable, non-release scope.
2. A hypothetical hardening concern, defense-in-depth improvement, unobserved
   local helper exposure, or broader integration mismatch does not block that
   basic acceptance unless current evidence shows it can disclose retained
   plaintext, delete or corrupt data outside the owned disposable root, perform
   an unauthorized external action, or invalidate the claimed result.
3. After basic acceptance, run independent code, security, and architecture
   review. Material residual risk becomes a proposed gate or correction only
   after its affected scenario and tradeoff are explicit.
4. When a material choice is genuinely ambiguous, record
   `HUMAN_REVIEW_REQUIRED`, ask the responsible user, and pause only the
   affected decision. Automation must not invent a blocking gate or silently
   choose a product/security tradeoff on the user's behalf.
5. Basic acceptance is not merge, release, deployment, public exposure, or
   long-term production approval; those later actions retain their own gates.

## Engineering Tool Policy Revocation

The responsible user revoked the version 1.8 special engineering-tool
constitution. It no longer creates a tool ban, a special scoped-open gate, or a
tool-specific evidence-invalidity rule for any current or future window.

The amendment sequence is retained exactly for auditability:

- At `2026-08-11T06:32:41.387Z` / `2026-08-11T14:32:41+08:00`, direct user
  text `解除禁用所有外部工具` validated the intermediate
  `ALL_EXTERNAL_TOOLS_ALLOWED` lift for
  `EXISTING_AUTHORIZED_PROJECT_TASKS_ONLY`. Its canonicalization is
  `RFC8785-JCS-UTF8` and its `lifted_scope_sha256` is
  `be9acacd1b3bb0a25519f7d7ae12285143531f2bd7229ce69d73abc87d116e31`.
- At `2026-08-11T06:37:17.362Z` / `2026-08-11T14:37:17+08:00`, the later
  direct user text `也解除憲法 以及其他窗口 對於外部工具的使用恢復正常`
  revoked the special engineering-tool constitution itself and superseded the
  need for `policy_lift_authority_ref`, `PAUSED_BY_TOOL_POLICY`, or any special
  scoped-open gate. Both receipts came from source thread
  `019fef39-6c03-76f0-9115-0171c7d44f10`; the intermediate receipt identifies
  user message `item-164`.

Current engineering windows therefore use shell, PowerShell, Git, Cargo, npm,
Node.js, PostgreSQL/MCP tooling, browser/web capabilities, plugins, and other
available tools normally when they are relevant to an existing authorized
project task. Normal Codex and platform permissions, `AGENTS.md`, repository
instructions, task scope, and safety rules remain controlling. This amendment
does not authorize secret disclosure, unscoped destructive action, payment or
account changes without explicit authority, or merge, deployment, release, or
default-branch changes without explicit authority.

The amendment itself is not a `PASS`. Earlier dynamic evidence remains
historical when time or candidate drift makes it stale and requires current
reverification for a current claim, but it is not invalid or paused merely
because an external tool produced it. The product's microkernel, One Writer,
credential, exact-tool, GH-9 non-composition, and TASK-037 isolation rules are
unchanged.

## Provider-Neutral Correction Rule

Validation is never removed merely because a provider returned incomplete or
invalid data. This rule applies to Codex and to every future AI provider:

1. The owning adapter identifies only the missing or invalid closed fields and
   makes one bounded correction request to the same authorized provider. The
   request cannot widen the Task, tool schema, credential scope, writer
   authority, path, command, or external action.
2. The corrected response passes through the complete original validation
   again. Uncorrected, still-invalid, substituted, or ambiguous output remains
   a typed failure and never becomes accepted evidence.
3. No new provider key, permission change, plugin, or authority is created
   merely to perform the bounded correction. A genuinely ambiguous product or
   authority choice remains a typed unresolved decision for the responsible
   authority.
4. Non-AI machine data is not sent to an AI for repair. Invalid byte sequences
   in owned diagnostic logs use a deterministic canonical UTF-8 copy while the
   immutable raw byte count and digest, replacement count, and canonical digest
   are retained. The canonical text must still pass secret scanning, JSON
   parsing, and every exact lifecycle check before it can influence a decision.

## Invariants

1. Exactly one normal composition root selects concrete implementations.
2. Orchestrator owns effect order; `latticed` and its adapters do not reorder,
   skip, or synthesize delivery stages.
3. Concrete adapters are constructed at the edge, implement typed Ports, and
   never call one another.
4. Canonical `latticed` MCP enumeration contains exactly the four approved
   names. Delivery schemas remain zero-parameter; task schemas remain closed to
   the one approved intent/`client_request_id` and returned `task_ref`.
   Alternate `lattice-full-chain` is an observer only: it exposes the two
   delivery names, cannot dispatch Delivery Run, and never exposes or dispatches
   either task tool.
5. No MCP request can carry shell, SQL, path, credential, provider, arbitrary
   task text, process, Git/test command, actor/session authority, lease, fence,
   or writable Codex-thread data.
6. MCP calls reach the same `FullChainService` / Orchestrator composition as the
   normal `latticed` action. The MCP adapter is not a second gateway,
   orchestrator, task truth, provider registry, or queue owner.
7. `lattice-runtime delivery-run` cannot execute an official Codex writer or
   mint MCP/tunnel ingress evidence. Its scripted fixture is visibly fake and
   cannot substitute for canonical `latticed` acceptance.
   `lattice-full-chain` likewise cannot enter an official writer path; its
   retained Delivery Run name returns a fixed denial before service effects.
8. Startup, adapter, protocol, connector-snapshot, test, Git, database, or
   evidence ambiguity fails closed and never reports success.
9. Scripted protocol evidence remains visibly distinct from an official Codex
   app-server live turn and from a real ChatGPT invocation.
10. Graphify executable, source repository, commit, staging root, fixed memory
    query, timeout, and database configuration are process-owned and
    digest-bound. They never enter MCP schemas or echo secrets.
11. The server-owned Task Spec digest is the sole task binding; legacy fixed
    submission, profile/run, or synthetic authority digests cannot substitute.
12. Task lifecycle, exact idempotency, audit, lease/fencing, and status are
    PostgreSQL-backed and fail closed when their independently verified current
    heads are unavailable.
13. Public task status is an allowlist of typed state/disposition and digests;
    it contains no raw spec, prompt, diff, command, path, secret, lease/fence,
    child output, Reflection candidate, or database detail.
14. The Task lifecycle edge wrapper cannot construct event/receipt fragments,
    bypass Task Domain validation, or treat a PostgreSQL row as authoritative
    before Task Ledger replay/current-head verification.
15. The fixed canary cannot fabricate a live Project Registry authority or be
    generalized into project-selectable/free-form work before durable Registry
    currentness and normal Policy composition exist.
16. Missing, active-at-completion, or physically corrupt Writer Lease history
    cannot be downgraded to a valid terminal Task status or recovery path.
17. Tool discovery never implies invocation; local invocation never implies
    ChatGPT invocation; same-session read never implies cross-session durable
    replay.
18. The absence of a ChatGPT platform capability is reported as a fixed blocker
    and never used to justify a broader or less safe MCP surface.
19. Ports 1.9 Reflection traits are compile-time capabilities only under this
    version. No production queue, Hermes execution, or unattended worker is
    composed by implication.
20. Provider process extraction follows ADR-024: Port first, protocol second,
    with no moved kernel authority and a written complexity justification.
21. `CONTROL_PLANE_API_KEY` exists only inside the approved managed tunnel
    launch process -> pinned tunnel-client -> canonical `latticed` boundary.
    It is removed before every later child spawn and is absent from workspace,
    Git, logs, database/audit rows, results, and retained evidence.
22. Basic controlled acceptance precedes optional hardening. Only a currently
    demonstrated safety or result-integrity failure blocks the acceptance run;
    review-discovered residual risks are routed to the responsible user before
    a new gate or correction is imposed.
23. Every current or future AI provider receives at most one bounded correction
    request for invalid or incomplete closed output, followed by the complete
    unchanged validator. Correction grants no new scope or authority and cannot
    convert ambiguity into success.
24. Non-AI diagnostic canonicalization is deterministic and provenance-bound.
    Raw content is not rewritten in place; canonical text cannot affect
    lifecycle or acceptance unless secret scanning, JSON parsing, and exact
    classifier checks all pass.

## Allowed Dependencies

- `lattice-contracts` 1.12, `lattice-ports` 1.9,
  `orchestrator-runtime` 2.3, and Writer Lease 1.1 public APIs.
- Concrete Codex, PostgreSQL Task Ledger, bounded workspace/Git, and fixed-test
  adapters required by TASK-032, only for construction and Port
  implementation.
- Concrete exact-snapshot, Graphify 1.0, and pure Codebase Memory 1.0 adapters
  required by TASK-033, only for construction and Port implementation. The
  proposed independent PostgreSQL Memory extension adapter cannot be composed
  until its owning module receives an explicitly approved versioned amendment.
- Bounded stdio/JSON/MCP framing, Secure MCP Tunnel process configuration,
  hashing, timeout, and diagnostics libraries required at the application edge.
- Concrete PostgreSQL Task Ledger and PostgreSQL Writer Lease 1.0 adapters only
  for construction behind their typed boundaries.

## Forbidden Dependencies

- Orchestrator internals, direct policy/domain mutation, OpenClaw or ChatGPT SDK
  as a second control core, Graphify/Hermes/Memory mutation, Guardian trust
  roots, deployment/publication/payment code, or companion/playmate website
  code.
- Arbitrary shell/SQL execution, caller-selected filesystem/product paths,
  credential payloads, dynamic provider/tool registration, or an alternate
  durable ledger.
- Adapter-to-adapter calls or reverse dependencies from an adapter into the
  composition root.
- Production composition of GH-9 Reflection queue/history/candidate Ports,
  global Reflection discovery, or unattended workers without a later versioned
  constitution.

## Failure, Compatibility, And Migration

Unknown startup configuration, connector snapshot, extra MCP arguments,
duplicate/unknown tools, adapter construction failure, malformed framing,
interrupted output, or an uncertain downstream effect returns a bounded typed
failure or reconciliation result. No permissive fallback or generic command
surface is allowed.

Version 1.0 is implemented in the existing `apps/lattice-runtime` package. The
`latticed` executable is the canonical normal entry; the existing
`lattice-runtime` name remains a compatibility wrapper over the same
composition. Removing or behaviorally diverging that wrapper requires a
versioned migration decision and compatibility evidence.

Version 1.2 added two bounded task tools and one fixed server-derived MCP actor.
It did not add a second listener, per-human identity claim, generic task
surface, alternate task store, or direct writer control. Clients that do not
refresh discovery may retain stale tool snapshots; those snapshots are not
current acceptance evidence.

Version 1.3 closed the legacy `lattice-runtime delivery-run` official lane. The
command remains only for the exact repository-owned scripted fixture and fails
closed for `OFFICIAL_CODEX_APP_SERVER`.

Version 1.4 restricted alternate `lattice-full-chain` to a read-only delivery
observer and reserved all official mutation plus task control for canonical
`latticed`.

Version 1.5 changes governance priority and evidence semantics without adding a
new runtime tool or widening a schema. Real ChatGPT discovery, invocation,
bounded Submit, and cross-session durable Status are now explicit P0 gates.
Ports 1.9 is accepted as a compile-time dependency, but GH-9 Reflection Ports
remain uncomposed. OpenClaw remains fallback until a later explicit promotion.

Version 1.6 records the responsible user's credential-boundary decision for
the fixed P0 profile. The direct tunnel-client to canonical `latticed` stdio
child shares the process-private control-plane key; every downstream child and
output surface remains credential-closed. The retained key's OpenAI Platform
`All` permission is user-owned and immutable to automation until explicitly
superseded. No tool, schema, Port, provider, gateway, or writer authority is
added.

Version 1.7 keeps validation intact while adding one provider-neutral bounded
correction attempt for incomplete or invalid AI output. It also records the
responsible user's deterministic choice for non-AI diagnostic bytes: preserve
raw provenance, create a secret-scanned canonical UTF-8 copy, and retain all
JSON and lifecycle checks. No provider, key, permission, tool, schema, writer,
or product authority is added.

Version 1.8 introduced a special engineering-tool constitution that prohibited
external processes and assigned tool-policy-specific evidence states. Version
1.9 records the responsible user's later direct revocation of that special
constitution. Normal task-scoped tool use and evidence rules resume; the
product architecture and protected action boundaries do not change.

## Acceptance Gates

| Gate | Evidence | Owner | Required for merge |
|---|---|---|---|
| Composition direction | Cargo metadata proves orchestrator has no concrete dependency and only `latticed` selects adapters | Architecture review | yes |
| MCP tool closure | exact four-tool list; unchanged empty delivery schemas; closed task schemas and unknown/additional-property rejection | Engineering | yes |
| Restricted input | shell/SQL/path/credential/provider/task-text/actor/session/lease/fence/writable-thread matrix is rejected before dispatch | Security review | yes |
| One Gateway | both task tools invoke the same `FullChainService` / Orchestrator composition; MCP has no direct database/Codex/Git call path | Architecture review | yes |
| Fixed identity | process profile supplies the actor/audit binding; tunnel/local commitments cannot substitute; hostile `clientInfo`/arguments grant no authority | Security review | yes |
| Tunnel credential boundary | sentinel is present only in managed launcher, pinned tunnel-client, and canonical `latticed`; absent from Codex/downstream children and all output surfaces; Platform permission remains user-required `All` | Security + integration review | yes for ChatGPT P0 |
| Durable task control | Task creation/idempotency/audit/status replay from PostgreSQL with fresh-process equality | Engineering | yes |
| Writer authority | real PostgreSQL lease/fencing/current-head evidence; no fake/synthetic production path | Security review | yes |
| Legacy command isolation | `lattice-runtime delivery-run` accepts only the exact scripted fixture; official Codex and MCP/tunnel provenance fail before effects | Compatibility review | yes |
| Delivery acceptance | official Codex turn, isolated scope/test/commit, durable outcome and separate restart/status replay | Engineering | yes |
| ChatGPT discovery | real current ChatGPT connector snapshot shows the exact four tools and schemas | Integration review | yes for ChatGPT P0 |
| ChatGPT read round-trip | real ChatGPT request invokes a read-only tool through tunnel and canonical `latticed` | Integration review | yes for ChatGPT P0 |
| ChatGPT bounded Submit | real ChatGPT request invokes only the closed canary, or a fixed current platform blocker is retained | Security + integration review | yes for write claim |
| ChatGPT cross-session durable Status | separate new ChatGPT request/session returns identical PostgreSQL-backed result with zero repeated effects | Integration review | yes for durable-control claim |
| Reflection non-composition | tool list, composition tests, and dependency review prove GH-9 Reflection Ports are not silently active | Architecture review | yes |
| Failure closure | startup/framing/adapter/timeout/platform-capability/unknown-effect cases never report success | Engineering | yes |

## Change Policy

Composition ownership, tool names or schemas, compatibility-entry behavior,
adapter/provider dependency direction, credential boundary, gateway separation,
connector acceptance semantics, or success/failure meaning require a versioned
constitution amendment, SPEC/ADR trace, architecture review, and
responsible-user approval. This constitution cannot be weakened merely to
excuse implementation drift or a ChatGPT platform limitation.

## Amendment History

| Version | Date | Decision reference | Summary | Approver |
|---|---|---|---|---|
| 1.0 | 2026-08-05 | SPEC-002 v25, ADR-021, TASK-032 | Sole normal composition root in `apps/lattice-runtime`, two fixed zero-parameter MCP tools, and one shared `lattice-runtime` compatibility wrapper | User approval in preceding implementation window |
| 1.1 | 2026-08-05 | SPEC-002 v26, ADR-022, TASK-033 | Compose the exact Graphify/PostgreSQL Codebase Memory continuation while preserving the same two-tool MCP boundary | User TASK-033 direction |
| 1.2 | 2026-08-09 | SPEC-003 v3, ADR-023, TASK-038 | Add bounded Task Submit/Status through the same Gateway with a fixed server-owned actor, Task Spec digest unity, PostgreSQL task truth, and real writer lease/fencing | User TASK-038-first direction |
| 1.3 | 2026-08-10 | SPEC-003 v3, ADR-023 security correction, TASK-038 | Make canonical `latticed` the sole official writer entry and restrict the legacy CLI delivery command to the exact visibly scripted fixture | User TASK-038-first security boundary |
| 1.4 | 2026-08-10 | SPEC-003 v4, ADR-023 alternate-entry correction, TASK-038 | Restrict alternate `lattice-full-chain` to a read-only delivery observer and reserve all official mutation plus task control for canonical `latticed` | User TASK-038-first One Writer boundary |
| 1.5 | 2026-08-11 | ADR-024, Issues #4/#5/#9/#10 | Make real ChatGPT round-trip the P0 product gate, adopt microkernel + Port-first/MCP-provider boundaries, preserve four closed tools, and keep GH-9 Reflection uncomposed until later authorization | Responsible user architecture amendment |
| 1.6 | 2026-08-11 | SPEC-003 v5, ADR-023 credential/acceptance order, TASK-038 | Approve direct process-private key inheritance from pinned tunnel-client to canonical `latticed`, preserve the user-required Platform `All` setting, and require basic controlled acceptance before optional hardening with material ambiguity routed to human review | Responsible user explicit direction |
| 1.7 | 2026-08-11 | Responsible-user provider-neutral correction and Q-001=A direction | Require one bounded correction and full revalidation for invalid/incomplete output from Codex or any future AI provider; use deterministic provenance-bound UTF-8 canonicalization rather than AI repair for non-AI logs | Responsible user explicit direction |
| 1.8 | 2026-08-11 | Responsible-user constitutional engineering tool-policy amendment | Restrict every current and future engineering window to Codex App built-in thread and internal non-external-process capabilities; classify earlier output as historical pre-policy evidence; require `PAUSED_BY_TOOL_POLICY` when compliant evidence is unavailable; invalidate violating evidence for acceptance/publication/completion; reserve future changes to explicit user amendment | Responsible user explicit constitutional amendment |
| 1.9 | 2026-08-11 | Direct-user tool lift and special engineering-tool constitution revocation, effective 06:32:41.387Z then 06:37:17.362Z | Revoke the version 1.8 external-tool prohibition and all special scoped-open/tool-evidence gates; restore normal task-authorized tool use under Codex/platform/repository rules while preserving secret, destructive, payment/account, merge/deploy/release/default-branch, microkernel, One Writer, credential, GH-9, and TASK-037 boundaries; require current evidence for current claims and do not treat revocation itself as PASS | Responsible user direct amendment |
