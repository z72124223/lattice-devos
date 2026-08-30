# ADR-023: Bounded MCP Task Dispatch And PostgreSQL Writer Lease

- Status: accepted for TASK-038 Phase 2, amended for Phase 3 general task
  intake, and amended for the 2026-08-30 pre-deployment historical-ingress
  compatibility repair; each implementation/acceptance claim remains
  evidence-dependent
- Date: 2026-08-09
- Decision owner: user
- Related: SPEC-002 v34, SPEC-003 v5, ADR-005, ADR-006, ADR-012,
  ADR-015, ADR-017, ADR-019, ADR-021, ADR-022, TASK-037, TASK-038,
  TASK-076

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

## TASK-076 Writer Lease V2 Bridge Amendment

The v1 PostgreSQL extension remains immutable. Postgres Writer Lease 1.1 adds
one append-only v2 successor so the accepted global-v3/Memory-v2/v1 database
can reach schema v5 without losing or rehashing commands, transitions,
receipts, snapshots, checkpoints, lease revisions, or fencing high-water.

The sole accepted sequence is:

`G3_M2_W1_CURRENT -> G3_M2_W2_BRIDGE ->
G5_M2_W2_BRIDGE_PENDING -> G5_M3_W2_BRIDGE_PENDING ->
G5_M3_W2_CURRENT`.

Only the Writer owner creates the v2 bridge or final rebind. Store advances
only the global profile, and Memory advances only its extension after exact
companion verification. All three administrative runners acquire global,
Memory, and Writer transaction locks in that order. Both schema-v5 pending
states reject runtime and fenced Task Ledger writes. The G3/M2 v2 bridge is
likewise runtime-quarantined because neither the historical v1 bind/load pair
nor the final-only v2 pair is valid there. Fresh global-v5/Memory-v3
installation produces the same final catalog/current identity with a truthful
one-row fresh history; an upgrade retains the exact three-row v1/bridge/rebind
history.

The three state-mutation locks remain transaction-scoped
`pg_advisory_xact_lock(bigint)` acquisitions in global, Memory, Writer order.
To serialize the complete Writer apply boundary across its bounded
serialization retries, the Writer adapter additionally holds the same global
key through a nonblocking session `pg_try_advisory_lock(bigint)` gate. Gate
acquire and release each run in a short transaction under
`SET LOCAL ROLE lattice_migrator`; commit restores the NOINHERIT login role,
and success, error, and Drop paths release the session gate. Postgres Store
owns the protected-function ACL closure, not the gate or Writer semantics: no
LOGIN receives either function directly, `lattice_migrator` alone receives
the two exact non-grantable bigint acquisition grants, and the other fourteen
acquisition overloads remain denied after role selection.

For current schema-v5/Writer-v2 profiles, this amendment supersedes only
ADR-017's earlier statement that exactly one post-role advisory acquisition
grant exists. ADR-017 remains the immutable decision record for its historical
profile; the second exact grant is unavailable to that profile and becomes
valid only under the SPEC-002 v34 / Postgres Store 1.9 catalog closure.

Writer v2 adds only the two runtime successors required to replace v1
functions that hard-bound ledger ordinal 1. The other five v1 runtime
functions, including the fixed 15-scalar current-authority assertion, retain
their signatures and semantics. No MCP, Task Spec, lease transition, fencing,
credential, public-network, or provider/model contract changes.

## Phase 3 General Task Intake Amendment

This amendment is authorized on 2026-08-26 and supersedes only the Phase 2
statements that made `CONTROLLED_CODEX_CANARY` the sole Task Submit intent and
forbade every project selector or general objective. The canary contract and
its governed writer workflow remain compatible. This amendment does not widen
the execution, approval, payment, external-action, merge, deployment, or
release authority of either MCP tool.

### Registered-project resolution and authority boundary

`lattice_task_submit` may now accept one bounded natural-language `objective`
(with the general-task `intent` spelling retained as a compatibility alias),
one bounded `client_request_id`, and at most one locator: an exact Control
project ID or exact project display name. The caller never supplies a path,
repository identity, Registry receipt, snapshot, Task Spec, command, process,
credential, approval, lease, or execution setting.

The Control Project Catalog is only `CONTROL_LOCAL_CATALOG` locator data. Its
`registry_authority` remains `NONE`; a Control row, name, path, or UUID is not
itself formal project authority. The composition root may use one uniquely
matched, readable Control row only as a locator. It performs a bounded live
filesystem/Git observation between two exact eligible Control catalog-projection
reads, ignores unrelated Control status-field and legacy-row drift, and rejects
eligible catalog drift before resolving or registering that physical identity through the
existing PostgreSQL-backed Project Registry and reloads the formal record.
Only an independently replay-verified, current `ACTIVE` Project Registry
authority receipt and snapshot may be bound to the task. An absent, ambiguous,
unreadable, drifted, suspended, or substituted project fails with a typed
repairable error. Arbitrary or caller-selected unregistered paths are never
accepted.

When no selector is supplied, admission succeeds only if the eligible Control
catalog yields exactly one project. Name matching is exact and must also be
unique. LATTICE never guesses a project from conversational context, the
current directory, the first catalog row, or a legacy row.

### Durable create-only task

Task Ledger owns the versioned `TaskSubmissionEnvelope`, its canonical digest,
the public `task_ref`, and the create-only `GENERAL_TASK_INTAKE_V1`
Task-created marker. The envelope binds the process-owned ingress,
`client_request_id`, exact objective, retained project display name, formal
Project Registry authority-receipt digest, and a complete
`GENERAL_TASK_INTAKE` Task Ledger stream identity. That identity has a neutral
intake digest and deliberately has no Task Spec digest or accounting currency.
Postgres Store persists the shared ingress claim, envelope, and matching
`TASK_CREATED` append in the same `SERIALIZABLE` transaction. The fixed record
path also rechecks the exact current Project Registry row and authority receipt
inside that transaction. None of these rows is a Control SQLite shadow record,
second lifecycle table, or executable Task Spec.

Schema v7 is reached only by the explicit PostgreSQL bootstrap. Existing Writer
v3 SQL, rebind identity, checksum, and schema-v6 runtime profile remain
immutable; the append-only Writer v4 successor stages `V6V4Bridge`, and Store's
v6-to-v7 transaction may invoke only the fixed Writer-owned
`writer_lease_rebind_v4()` to reach `V7V4Current`. Normal MCP startup and tool
calls never migrate or rebind the database. Before backfilling the shared
ingress namespace, migration fails closed if any historical canary command
contains a now-recognized secret-shaped `client_request_id`; it neither copies
that value into the new tables nor completes a partial v7 transition.

### Pre-deployment duplicate-history compatibility amendment

The 2026-08-30 deployment rehearsal proved that pre-v7 command IDs were unique
only inside a Task Ledger stream. Two distinct durable streams may therefore
both validly retain `mcp-submit:delivery-run-controlled-compatibility` even
though schema v7 makes `(ingress_id, client_request_id)` globally unique. They
are separate historical task identities and must not be deleted, merged,
renamed, rewritten, or collapsed to an arbitrary winner.

Migration `0008` classifies the complete verified historical candidate set by
that exact key. A singleton becomes the active ingress claim. Every member of
a group with cardinality greater than one is instead inserted into the
migrator-owned `task_ingress_historical_ambiguities` relation with its exact
stream, event, command, event digest, and command-request digest. The active
claim table receives no row for that key. Normal runtime has no direct table
privilege on the ambiguity relation, and each fixed read, prepare, and record
entrypoint rejects it with SQLSTATE `LTX01` and the static diagnostic
`LATTICE_TASK_INGRESS_HISTORICAL_AMBIGUOUS`, which the adapter maps to its
existing command-substitution category. New v7 claims remain globally unique;
this is compatibility metadata, not a second ingress namespace or a repair
workflow. A fourth fixed security-definer verifier returns only a boolean and
lets fresh Migrator, Runtime, Guardian, and ReadOnly sessions rederive the
singleton/duplicate classification without direct table access. Exact
relation/column/constraint/index/function/ACL signatures and that boolean
closure reject catalog, privilege, function, or lineage drift.

The corrected ordinal-8 identity is exact: SQL SHA-256
`a9059c74722dcbff5345a2732bf1c44f8f2dd682a5eecb57bda2f0d820e9d4a0`,
334,756 bytes, and global manifest SHA-256
`584a446464ab2f7ebd8b85543ba36a6d52b0a708502c39d2653b8814d84313f8`.
Writer v4 rebind and Memory's read-only v7 bootstrap profile are re-pinned to
that manifest without changing their semantic rows. A database that already
records a different ordinal-8 checksum remains a fail-closed manifest mismatch
and is never reinterpreted or bootstrapped by replaying the old candidate.
For the exact f252 schema-v6 bridge only, composition first invokes the
Writer-owned typed v4 apply to replace its verified legacy procedure, then
reclassifies the profile; Store remains call-only and starts v7 migration only
after the current Writer bridge is independently verified.

Release evidence must include a production-shaped v6 fixture with two distinct
streams sharing the historical key, post-DDL failure atomicity, exact lineage
and no-winner assertions, all three runtime denials, and an `AlreadyCurrent`
retry from a fresh database client with unchanged durable fingerprints. The
deployment gate additionally requires a verified physical backup plus isolated
restore/migration and rollback clones before the live database is changed.
Synthetic rows, the old failed release artifact, or a same-process no-op do not
satisfy those gates.

Orchestrator exposes one pure create-only path over the separate
`TaskIntakeLifecyclePort`. It admits the verified intake once, or returns its
exact replay, leaving the formal task in `DRAFT`. The intake has no autonomy
classification or receipt and cannot be passed to the Task-Spec lifecycle,
Policy, approval, Writer Lease, delivery, or execution ports. This is not a
Policy allow decision or bypass: no progression or external-effect operation
exists on the intake port. Any later planning, specification, tickets,
execution approval, agent start, mutation, integration, deployment, or release
remains a separate governed operation with its normal gates.

### Idempotency, validation, and status

General-task idempotency is scoped by the server-owned ingress plus
`client_request_id`. An exact retry with the same objective and formal project
binding returns the already retained `task_ref`, including from a fresh
process. Reusing the key with another objective or project is a typed command
substitution failure and never creates another task. The retained envelope is
loaded before attempting to infer a new project so a restart cannot silently
retarget an exact retry. Canary and general submissions share this ingress-key
namespace, so reusing one key across the two modes is also substitution rather
than a second task.

The Task Ledger claim, task stream, command, event, and general envelope are
transactionally strict across concurrent processes: exactly one request kind
can own a key and no losing request creates a second task. If identical callers
collide while first registering or observing the formal project, the resolver
performs at most one complete fresh Control/Git/Registry pass, pinned to the
same initially selected project ID. It reuses an already identical active
formal observation, but never reuses a stale physical observation or retargets
a name/no-selector request to another project. If identical callers then
resolved two snapshots of the same formal project, the admission loser
fresh-loads the winning envelope and returns its `task_ref`. If the newer
snapshot has become current but its caller has not committed the envelope yet,
the stale caller may re-resolve that same effective project and retry admission
once, then reload the winner once more. It never sleeps, polls, retries a
different project, or replays a different objective. This phase does not
add a request-wide cross-process serialization guard around the earlier
Project Registry resolver. In the narrow race where a canary commits after a
general request's final preflight but before its Task Ledger append, the losing
general request may already have retained an independently valid live project
observation before returning the typed idempotency conflict. That observation
does not create or progress a task and grants no execution authority.
Eliminating even this auxiliary resolver effect requires a separately reviewed
scoped ingress guard and remains later hardening; preflight must not be
described as globally race-atomic.

Objective and project-name text must be non-empty, already NFC, trimmed, within
their byte bounds, and free of NUL/control characters and recognized secret
material. Client request IDs use the shared one-to-64-byte
`[A-Za-z0-9][A-Za-z0-9._:-]*` contract; they, project IDs, and the retained
formal project snapshot ID must also be free of the shared recognized secret
shapes before persistence or public projection. The Task Ledger, Store, and
public projection accept the Project Registry's closed maximum 159-byte
snapshot form (64-byte project ID, `:registry:`, 20-digit revision, colon, and
64-byte digest) and reject 160 bytes. The objective is retained only as task
data. It is never parsed or
executed as a shell command, SQL, filesystem path, permission, configuration,
credential, provider instruction, or approval.

General Task Submit and `lattice_task_status` return the closed
`lattice.task.status.v3` projection. It includes only the durable `task_ref`,
`SUBMITTED`/`DRAFT` disposition, Ledger-head digest, exact objective, Control
project ID/display name, formal project snapshot ID, and nullable terminal
result/failure fields. Status requires `task_ref`; the optional
`client_request_id` exists only for legacy canary compatibility. A fresh
process reconstructs v3 status from the verified PostgreSQL envelope and Task
Ledger stream without repeating project registration, task creation, or any
external effect. Existing canary results remain on their compatible v2
projection.

### Codex caller behavior

Codex should call general Task Submit when the user asks LATTICE to create,
record, track, or durably resume a project task and the project is already
registered. Codex returns the resulting `task_ref` and `task_state` to the
user. It must report the typed selector/registration error when no project is
uniquely eligible, and must not turn a short objective into a specification,
tickets, execution, agent dispatch, payment, external action, merge,
deployment, or release unless a later request and its own authorization gates
explicitly allow that operation.

## Acceptance Evidence Required

- Exact seven-tool discovery and closed-schema rejection matrices, including
  the general objective/selector variants and retained canary variant.
- Same-key exact retry, changed-objective/project/canary-mode substitution
  denial, and general-intake envelope/Ledger replay from PostgreSQL. Production
  Secure MCP Tunnel and local canonical acceptance commitments are distinct
  and cannot substitute.
- For the retained canary only, one spec digest across Gateway, Ledger, lease,
  Codex, verification/Git, and status evidence. General intake instead proves
  the distinct subject kind, neutral intake digest, and absence of Task Spec,
  currency, autonomy, Writer Lease, and effect rows.
- Concurrent acquire, monotonic non-reused fencing across restart, stale-fence
  rejection, heartbeat/release, and ambiguous-outcome reconciliation against
  PostgreSQL 17.
- A controlled Codex canary that changes only its template-owned scope, passes
  fixed verification, commits once, and records a durable terminal result.
- Fresh-process `lattice_task_status` equality with zero repeated external
  writer effects and zero Graphify/Hermes/Memory effects.
- Fresh-process general Task Submit/Status proves the same `task_ref`, exact
  objective, formal registered-project/snapshot binding, and `DRAFT` state
  across restart. Submit performs only the documented bounded Control,
  read-only Git-observation, Registry, and Task-Ledger effects; replay/Status
  repeats none of them and no step starts Codex/model execution, a Writer
  Lease, workspace/Git mutation, or downstream external action.
- No Fake/synthetic authority in production evidence and no secret-bearing
  field in schemas, normal results, logs, or retained audit data.
- No fake or caller-projected Project Registry fact. Control remains a locator;
  general intake requires durable Registry currentness, while any later task
  progression still requires its separately governed Policy/approval/writer
  composition.

No item above is recorded as passed by this ADR.
