# ADR-023: Bounded MCP Task Dispatch And PostgreSQL Writer Lease

- Status: accepted and implemented for TASK-038 Phase 2; canonical-local
  PostgreSQL/Codex/Git acceptance passed, real Secure MCP Tunnel / ChatGPT
  discovery and two-session invocation remain incomplete
- Date: 2026-08-09
- Decision owner: user
- Related: SPEC-003 v4, ADR-005, ADR-006, ADR-012, ADR-015, ADR-019,
  ADR-021, TASK-037, TASK-038

## Context

TASK-038 Phase 1 proved that ChatGPT can discover and invoke the two bounded
delivery tools through the official private Secure MCP Tunnel. It did not let
ChatGPT submit a normal LATTICE task, and it deliberately did not claim a
successful production execution.

The existing Rust contracts already contain typed Gateway `Submit` and
`Status` requests. The existing Task Domain owns Task Spec 2.1, Task Ledger
owns event/idempotency/replay semantics, and Orchestrator owns workflow order.
Adding an MCP-specific queue, database, or direct Codex controller would create
a second gateway, truth source, orchestrator, or writer.

The current fixed delivery path also cannot be promoted to formal task
dispatch merely by renaming it. Its delivery binding is not the Task Spec
digest, and its recorded authority is synthetic rather than a live PostgreSQL
Writer Lease. Formal Codex execution therefore requires a real lease repository
and monotonic fencing evidence before the writer is reachable.

## Decision

### One Gateway And Fixed Ingress Identity

`latticed` remains the sole normal composition root and service. It
adds exactly two bounded MCP tools:

- `lattice_task_submit`;
- `lattice_task_status`.

The MCP adapter maps these tools into the existing `FullChainService` and
injected Orchestrator composition. It does not call PostgreSQL, Codex, Git, or
a second workflow service directly.

The first production ingress identity is one server-owned fixed
tunnel/profile actor. Its actor, gateway instance, adapter identity, channel,
profile, and authority evidence come only from process-start configuration and
verified tunnel/runtime identity. MCP `clientInfo`, tool arguments, model text,
and caller-supplied actor/session fields are informational or rejected; none
can grant identity, policy, lease, approval, or writer authority. This decision
does not claim per-human ChatGPT identity.

### Closed Task Intent And Server-Owned Task Spec

Phase 2 admits exactly one public intent:

`CONTROLLED_CODEX_CANARY`

`lattice_task_submit` accepts only that closed intent and one bounded
`client_request_id` idempotency key. It accepts no free-form implementation
prompt, shell, SQL,
filesystem path, Git command, verification command, credential, provider
configuration, actor/session identity, lease, fencing token, or writable
thread identifier.

The server-owned template selects project, snapshot, repository/base,
repository-relative scope, fixed verification, capabilities, budgets,
approvals, workspace profile, Codex prompt template, and downstream profile.
It constructs and revalidates the complete Task Spec 2.1 through Task Domain
2.2 and uses that owner's canonical subject/document rather than rebuilding a
reduced carrier.
The resulting one `spec_digest` is authoritative across the Gateway binding,
Task Ledger stream identity, Writer Lease identity, Codex
request, verification/Git evidence, durable status, and downstream delivery
evidence. No alternate five-field document, profile/run digest, or synthetic
delivery identity may substitute for it.

TASK-038 does not invent a live Project Registry authority to make the normal
Policy engine return allow. The durable live Registry adapter remains absent;
therefore this first server-owned canary is not authority for exposing a
project selector, free-form intent, or any broader development template.

`lattice_task_status` accepts only the lowercase SHA-256 `task_ref` returned by
Submit. The reference selects an already admitted task inside the same fixed
project/profile; it grants no mutation or authority.

### PostgreSQL Truth, Idempotency, Rate, And Audit

PostgreSQL is the only live durable control-plane truth.

- Task Ledger records Task creation, legal state transitions, effect intents,
  outcomes, terminal receipts, exact command idempotency, and the fixed actor/
  profile audit binding through its verified append/replay semantics.
- The `latticed` Task lifecycle edge wrapper implements the injected
  `TaskLifecyclePort` by reusing the existing `PostgresTaskLedger`; it neither
  changes Postgres Store's semantic/API ownership nor constructs Ledger
  events/receipts outside the Task Ledger planner/replay boundary.
- An exact retry of the same `client_request_id` and request returns the same
  accepted task/receipt. Reuse with changed content is denied without another
  Codex effect.
- The first profile has exactly one server-owned
  `CONTROLLED_CODEX_CANARY` task subject. Admission and exact retry are durable
  audited PostgreSQL facts; a different key for that subject is denied before
  another Codex effect.
- Status is reconstructed from independently verified PostgreSQL heads,
  checkpoints, events, and receipts. A process cache or MCP session is never a
  status authority.

Task multiplicity/quota policy, arbitrary projects, or
per-human quotas requires a later versioned decision.

### Real Writer Lease And Fencing

Writer Lease 1.1 remains the semantic owner of lease transitions, complete
snapshot/checkpoint bytes, exact retry, recovery, and the repository trait.
The new `postgres-writer-lease` 1.0 adapter implements that trait through the
independent exact extension `db/extensions/writer-lease/v1.sql`.

The extension is not a global Store migration and does not change
`db/migrations/0001` through `0004`. Postgres Store 1.6 may recognize one exact
combined catalog/ACL profile for compatibility verification, but it neither
installs nor mutates Writer Lease state and does not acquire lease semantic
ownership.

Postgres Store 1.6 is also the physical transaction boundary for Task Ledger
appends. Its fenced append may invoke only the fixed 15-scalar
`writer_lease_assert_current_v1` predicate inside the same `SERIALIZABLE`
transaction and before the Ledger/Store mutation. The Store adapter receives
an already typed complete authority head from Contracts; it does not depend on
the Writer Lease crates, construct or parse lease state, call the repository,
or plan a transition. This narrow atomic assertion is transaction enforcement,
not persistence or semantic ownership.

Formal Codex dispatch requires all of the following:

1. a Task-Spec-bound live lease is acquired through the injected repository;
2. the independently loaded authority head is current;
3. the monotonically increasing fencing token and exact worktree/process claim
   are bound into the writer request and evidence;
4. heartbeat/currentness checks remain valid at every mutation boundary;
5. verification and Git commit require the same current lease/fence;
6. terminal release or typed reconciliation is durably recorded.

`FakeWriterLease`, a hard-coded authority head, a synthetic epoch/fence, or a
receipt projected from itself is forbidden in production acceptance.

### Workflow Ownership

Orchestrator Runtime 2.3 alone orders the bounded task workflow through
injected ports and the Writer Lease repository:

```text
Gateway Submit
  -> TaskSpec validation
  -> PostgreSQL TaskCreated/admission audit
  -> PostgreSQL Writer Lease acquire/currentness
  -> bounded workspace
  -> Codex writer
  -> scope and fixed verification
  -> local Git commit
  -> lease release or reconciliation
  -> durable outcome/status
```

The first failed or uncertain gate suppresses all later effects. TASK-038 may
prove bounded Submit/Status and the controlled Codex canary before TASK-037's
separate production `Hermes -> Memory -> Status` repair is resumed; that order
does not waive the final downstream production gate. The Task capability is
`WriterOnly` and must leave Graphify/Hermes/Memory footprints at zero. The
legacy MCP Delivery Run compatibility tool enters the same governed writer path,
then may run its downstream continuation only after Task completion and Writer
Lease release.

The fixed canary has a 300-second Task budget, a 30-second finalization reserve,
and a 600-second Writer Lease TTL. Fresh execution outside that bound is denied
at composition. A future longer-running profile requires heartbeat, governed
child interruption, and orphan reconciliation before it can be exposed.

### Public Status Projection And Fresh Replay

The MCP result is an allowlisted projection only. It may expose schema/tool
version, `task_ref`, fixed intent, typed state/disposition, `spec_digest`,
ledger-head digest, observation/command receipt digest, and typed bounded
verification/Git/downstream disposition when those facts are durably present.
It never exposes raw Task Spec bytes, prompt, source diff, path, command, SQL,
environment, credential, actor/session secret, lease token, fencing token,
process output, child stderr, or database detail.

A new process and new MCP request/session must be able to load the same task
handle and return the same durable terminal projection without rerunning
Codex, verification, Git, Graphify, Hermes, or Memory.

## Dependency Direction

```text
MCP transport -> latticed -> FullChainService -> orchestrator-runtime
Lattice Ports -> Contracts + Task Domain 2.2's closed TaskState only
orchestrator-runtime -> Task Domain / lattice-ports / writer-lease
lattice-postgres-store -> Task Ledger planner/replay + fixed current-authority assertion
lattice-postgres-writer-lease -> Writer Lease planner/replay
concrete adapters -> their owning typed contracts/traits
```

No database adapter depends on Orchestrator or another concrete adapter.
Neither MCP nor `latticed` owns task transition or lease semantics.

## Compatibility And Migration

The existing `lattice_delivery_run` and `lattice_delivery_status` names and
zero-argument schemas remain compatible on canonical `latticed`. Its tool list
expands from two to four only for clients that refresh discovery.

The alternate `lattice-full-chain` executable remains a legacy observer, not a
second official writer. Its catalog contains the two delivery names only;
Delivery Run is fixed-denied before service dispatch, Status remains read-only,
and both task names are unknown in legacy and stateless MCP. This executable
restriction completes the same sole-entry security decision rather than
creating a new orchestration path.

The legacy `lattice-runtime delivery-run` command is not an MCP ingress and
cannot accurately inherit either the secure-tunnel or local canonical MCP
actor. It also accepts CLI path/workspace parameters that cannot be allowed to
select a second official writer composition. TASK-038 therefore restricts
that command to its exact, visibly scripted repository fixture and rejects
`OFFICIAL_CODEX_APP_SERVER` before identity, database, workspace, or process
effects. Canonical `latticed` is the sole official writer entry. This is a
versioned security narrowing of the CLI wrapper only; MCP delivery tool names,
schemas, routing, and durable evidence remain unchanged.

Writer Lease installation is an explicit independently verified extension
operation. Missing, partial, drifted, wrong-owner, or wrong-ACL extension state
fails startup/admission closed. No automatic global migration or permissive
fallback is allowed.

## Consequences

- ChatGPT gains a useful but deliberately narrow typed task capability without
  acquiring a shell, database, filesystem, Git, credential, lease, or Codex
  control surface.
- The first profile proves the real governance path before arbitrary
  development intents are considered.
- PostgreSQL and pure domain owners remain authoritative; the implementation
  cost includes a real Writer Lease adapter, durable admission evidence,
  and fresh-process replay tests.
- Per-human identity, arbitrary project selection, generic task text, broader
  templates, and production TASK-037 downstream repair remain explicit later
  work.

## Acceptance Evidence Required

- Exact four-tool discovery and closed-schema rejection matrices.
- Same-key exact retry, different-key substitution denial, and fixed-profile
  audit replay from PostgreSQL. Production Secure MCP Tunnel and local
  canonical acceptance commitments are distinct and cannot substitute.
- One spec digest across Gateway, Ledger, lease, Codex, verification/Git, and
  status evidence.
- Concurrent acquire, monotonic non-reused fencing across restart, stale-fence
  rejection, heartbeat/release, and ambiguous-outcome reconciliation against
  PostgreSQL 17.
- A controlled Codex canary that changes only its template-owned scope, passes
  fixed verification, commits once, and records a durable terminal result.
- Fresh-process `lattice_task_status` equality with zero repeated external
  writer effects and zero Graphify/Hermes/Memory effects.
- No Fake/synthetic authority in production evidence and no secret-bearing
  field in schemas, normal results, logs, or retained audit data.
- No fake or caller-projected Project Registry fact; broader task admission
  remains closed until durable Registry currentness and Policy composition
  exist.

No item above is recorded as passed by this ADR.
