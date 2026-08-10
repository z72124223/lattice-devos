---
spec_id: SPEC-002
status: ready
version: 27
supersedes_for_new_work: SPEC-001
modules:
  - module_id: lattice-cjson
    constitution_version: 1.0
  - module_id: task-domain
    constitution_version: 2.1
  - module_id: task-ledger
    constitution_version: 2.1
  - module_id: policy-engine
    constitution_version: 2.6
  - module_id: project-registry
    constitution_version: 1.2
  - module_id: writer-lease
    constitution_version: 1.0
  - module_id: workspace-git
    constitution_version: 2.0
  - module_id: scope-check
    constitution_version: 1.1
  - module_id: orchestrator-runtime
    constitution_version: 2.2
  - module_id: latticed
    constitution_version: 1.1
  - module_id: openclaw-adapter
    constitution_version: 2.0
  - module_id: gateway-ipc
    constitution_version: 1.1
  - module_id: approval-verifier
    constitution_version: 1.0
  - module_id: postgres-store
    constitution_version: 1.4
  - module_id: artifact-store
    constitution_version: 1.0
  - module_id: codex-adapter
    constitution_version: 1.1
  - module_id: review-runtime
    constitution_version: 1.0
  - module_id: graphify-adapter
    constitution_version: 1.1
  - module_id: hermes-adapter
    constitution_version: 1.0
  - module_id: codebase-memory
    constitution_version: 1.0
  - module_id: self-upgrade-guardian
    constitution_version: 1.0
  - module_id: lattice-core-bootstrap
    constitution_version: 1.0
  - module_id: lattice-cli
    constitution_version: 1.0
  - module_id: lattice-contracts
    constitution_version: 1.11
  - module_id: lattice-ports
    constitution_version: 1.7
---

# Autonomous Development Platform

## Problem

The V1 prototype proves several local workflow invariants, but it is not the
requested end product. It uses a Node.js core and file-based control state,
excludes real Codex execution, Graphify, Hermes, PostgreSQL, and long-term
memory, and was originally framed around an example project.

LATTICE needs to become a project-agnostic platform on the user's computer that
can plan, implement, verify, remember, and improve development work without
creating duplicate authorities or an unconstrained self-modifying agent.

## Intended Behavior

### General project boundary

- LATTICE accepts work only for an explicitly registered project identity and
  canonical root.
- Project registration binds canonical Windows path, repository identity,
  filesystem/file identity, and lifecycle state. Duplicate aliases, junction
  ambiguity, moved repositories, or identity drift block until reconciled.
- A Task Packet binds project identity, base commit, goal, non-goals, allowed
  paths/operations, acceptance criteria, capability budget, runtime profile,
  approval requirements, and immutable specification hash.
- No user project is hard-coded into policy, roles, schemas, tests, or active
  documentation.
- Access outside the registered project and LATTICE-owned task workspace
  defaults to deny.

### One Gateway

- OpenClaw is the normal human surface for `/lattice` submit, plan, status,
  approve, reject, and stop actions.
- Its plugin authenticates and translates typed commands to a versioned local
  LATTICE IPC contract.
- It cannot send arbitrary SQL or shell requests and cannot directly access
  PostgreSQL, Git, a product worktree, or provider credentials.
- A recovery-only supervisor surface may report status, stop a daemon, or roll
  back a release. It cannot submit ordinary development tasks.
- Protected core-release approval uses a separate guardian-owned,
  OS-authenticated local administrative surface. OpenClaw may display or
  initiate it, but a normal gateway session/token cannot alone satisfy it.

### One Truth

- PostgreSQL is the sole durable control-plane authority.
- Task specs, events, command receipts, approval nonces, writer lease counters,
  lease projections, effect intents/outcomes, evidence references, component
  capabilities, memory promotion, and release activation are transactionally
  recorded.
- Current state is replayed or projected from versioned events and a verified
  stream head. A stale or inconsistent projection fails closed.
- Filesystem state, `LISTEN/NOTIFY`, queues, generated reports, Graphify graphs,
  Hermes memory, and transcripts are non-authoritative evidence, caches, or
  candidates.
- An intent event and outbox record are durable before an external side effect
  begins. Unknown effect outcome becomes reconciliation work, never assumed
  success.
- Capability observations include timestamp, binary identity/digest, exact
  version, generated schema hash when available, explicit feature-probe
  results, and expiry/freshness policy. Stale evidence fails closed.

### Task Ledger V2 semantic boundary

- A task event stream binds canonical project identity, immutable Registry
  snapshot, Task ID, Task Spec revision/hash, and accounting currency. A naked
  Task ID or caller-defined system-stream Boolean is not a stream identity.
- Version 2.0 separately domain-hashes stream ID, command request, event,
  stream head, command receipt, resource projection, resource observation, and
  resource receipt through `lattice-cjson-1`.
- Append supplies the complete expected stream head: fixed producer/version/
  runtime, identity, sequence, last event hash, resource revision/projection
  digest, and head digest. Sequence or predecessor alone is insufficient.
- Exact `(stream_id, command_id)` retry lookup occurs before stale-head
  evaluation. The same sanitized request returns the byte-identical terminal
  receipt after later stream advancement; changed content rejects. A new
  stale/mismatched request returns a stable denial receipt without changing the
  event, stream head, or resource projection.
- Each durable command row retains the complete canonical request source,
  request digest, storage key, and terminal receipt. This is required because a
  denied command has no event from which replay could reconstruct its request.
- Events use closed versioned kinds/outcomes, typed authoritative digests and
  resource snapshots, strict identifiers, canonical caller-supplied UTC time,
  and optional bounded sanitized diagnostics. Diagnostics are
  non-authoritative and never carry a transition, approval, or effect identity.
- Replay rejects unknown schema/event kinds, missing/reordered/duplicate/
  truncated records, cross-stream fields, hash/predecessor/sequence mismatch,
  orphaned receipts, or claimed head/resource-projection disagreement.
- Replay accepts only an explicitly untrusted complete persistence snapshot,
  validates raw events plus every appended/denied command record, and returns a
  typed reconstructed stream. Fake and future storage adapters use this same
  pure semantic boundary; it performs no I/O and proves no durability or
  authentication.
- Task Ledger owns chain replay and the resource projection. Task Domain alone
  owns legal task-state transitions. Future Orchestrator composition consumes
  both public contracts without a dependency edge between them.
- TASK-013 supplies only a deterministic `RuntimeKind::Fake` in-memory owner.
  It is neither durable restart evidence nor authority to perform a live
  effect. ADR-005 PostgreSQL atomicity remains a later gate.

### One Writer

- Only the Codex Implementer may create, modify, delete, or rename product
  files, and only while holding the current project lease and fencing token in
  a LATTICE-owned worktree.
- PostgreSQL stores a monotonically increasing `BIGINT` fencing counter and at
  most one active project lease. Overflow fails closed; tokens never wrap.
- PostgreSQL also stores the active daemon instance and non-wrapping `BIGINT`
  epoch. Except for guardian-only release/epoch procedures, every
  daemon-authorized durable mutation validates active instance, epoch, and
  runtime-admission mode in the same transaction, including task/event,
  registry, lease/outbox, artifact metadata, memory, review/capability, and
  ordinary approval changes.
- PostgreSQL runtime admission is `ACTIVE`, `DRAINING`, `CANARY`, `STOPPED`, or
  `RECONCILIATION_REQUIRED`. `DRAINING` rejects new task/lease/effect admission;
  `CANARY` permits only a guardian-reserved system health stream; the latter two
  stopped states reject all daemon mutations.
- A local filesystem/process lock may add defense in depth but cannot become a
  competing durable truth.
- Read-only planners, reviewers, Graphify, Hermes, the Integrator, and the
  upgrade guardian cannot obtain a product-code lease.
- The Integrator may perform authorized, non-conflicting Git metadata
  integration. A product-code conflict becomes a new Implementer task.

### Rust control core

- The Rust core owns domain contracts, policy, orchestration, transactional
  store ports, scope rules, process supervision, timeouts, cancellation,
  adapter capability verification, and normalized evidence.
- Pure domain/policy modules perform no I/O.
- `orchestrator-runtime` is a pure Rust effect coordinator. It receives typed
  delivery ports and can only order durable intent, bounded workspace
  preparation, Codex execution, workspace/test/Git verification, and durable
  outcome/receipt recording; it selects no concrete adapter and performs no
  direct I/O.
- `latticed` 1.0 is the sole normal composition root. The existing
  `apps/lattice-runtime` package implements it, selects concrete adapters, and
  retains `lattice-runtime` only as a compatibility wrapper over the same
  composition and state.
- The Codex App MCP stdio surface exposes exactly two zero-parameter tools,
  `lattice_delivery_run` and `lattice_delivery_status`. Their tool schemas
  accept no shell, SQL, path, credential, provider, or arbitrary task input.
  This surface is not a second general gateway; OpenClaw remains the normal
  human gateway.
- Provider-specific behavior remains behind versioned ports. External adapters
  never receive general PostgreSQL credentials.
- Project Registry, Writer Lease, Task Ledger, Codebase Memory, Artifact Store,
  Review Runtime, and Approval Verifier have separate domain ownership; the
  PostgreSQL adapter supplies physical persistence without deciding their
  legal transitions.
- Local IPC uses an authenticated OS-local transport and does not listen on a
  public interface by default.

### Artifact Store 1.0 boundary

- Artifact objects are content-addressed by exact SHA-256 bytes inside one
  project namespace: `(project_id, sha256)`. Equal bytes do not share an
  observable identity, reference set, path, or lifecycle across projects in
  1.0.
- Every available object has a positive signed-BIGINT-compatible generation
  and revision. Reintroduction after an exact sweep uses a higher generation,
  so stale reference or sweep evidence cannot target new bytes.
- Object bytes and uses are separate. Each immutable reference binds project,
  snapshot, task/revision/spec, attempt, request, object/generation,
  media/schema/bundle, producer/version/runtime/binary, adapter/version/binary,
  invocation/correlation/run/sequence/produced-at/payload, capability/input/config/
  evidence, Registry snapshot authority, effect claim, daemon
  instance/epoch/admission, capability-owner receipt/head, limit snapshot,
  purpose, and retention fields.
- Graphify, Hermes, Codex, Review Runtime, Guardian, a model, or a product
  repository may appear only as source provenance. Only fixed
  `lattice-artifact-store` receipts represent Artifact Store state, and those
  receipts grant no truth, review, memory, policy, code-write, approval, merge,
  activation, or release authority.
- Exact byte length and raw SHA-256 are verified before publication. Empty
  artifacts are valid; the hard object limit is 1 GiB with a lower configured
  task/store limit allowed. Hard manifest, per-object reference, per-task
  object/reference/active-byte/staging-byte, and per-project
  object/reference/unique-byte quotas also apply and update atomically. A
  caller cannot supply trusted counts or a `within_quota` Boolean.
- Initial publication/reference, retain, and release require a typed
  fixed-owner authority receipt plus an independently queried complete owner
  head bound to the exact action, project/task/object/generation/reference,
  owner record/revision/status, and fake/live runtime. A caller count, Boolean,
  producer string, or bare digest is never authority.
- Exact command retry precedes stale-head/time checks. Applied and denied
  receipts share one predecessor chain; replay and an independently retained
  checkpoint detect tamper, denial-tail loss, and coherent rollback.
- Reference release is terminal. Delete claim requires internally recomputed
  zero active references/quota projection, expired retention/grace, exact
  generation/current head, explicit database time, current daemon/root
  binding, and a typed fixed-owner sweep receipt plus independent owner head.
- `DELETE_CLAIMED` uses one exact token and blocks retain/normal read. Success
  reaches `DELETED`; verified no-effect may return to `AVAILABLE`; an unknown
  transaction/filesystem result enters `RECONCILIATION_REQUIRED` and cannot be
  guessed safe. Reconciliation requires exact metadata-plus-byte evidence.
- Project/store bytes count each non-deleted generation once; task bytes count
  one object once when that task has an active reference. Reference/read/
  staging/command/history counts are exact, and separate task/project/store
  quota aggregates update atomically with the object aggregate.
- `DELETE_CLAIMED`, `RECONCILIATION_REQUIRED`, and sealed orphans retain
  worst-case quota. Object quota releases only on verified `DELETED`; staging
  releases only after metadata publication or verified cleanup/reconciliation.
- Active-read acquire/release is typed, object-scoped, idempotent, and bounded.
  Lease expiry becomes delete-blocking `EXPIRED_SUSPECT` until verified holder
  or handle reconciliation.
- The pure Rust owner exposes no real filesystem unlink. PostgreSQL later
  serializes metadata/reference truth without redefining semantics; a separate
  owned-root filesystem adapter later performs staging, flush, atomic rename,
  verified read, link containment, and one-object cleanup mechanics.

### Policy V2 decision boundary

- Policy consumes a complete, already-constructed immutable Task Spec directly.
  A missing or partial subject is a typed denial, never an implicitly trusted
  object.
- Task Domain owns Task Spec, task state, risk, capability, network,
  deployment, approval-requirement, and check vocabulary. Policy owns the
  closed authority-role/action sets, deterministic allow/deny decisions, and
  stable reason codes.
- Task Spec budget schema `2.1` includes one canonical uppercase accounting
  currency. The external-cost ceiling, Task-Ledger resource fact, exact quote,
  and approval subject must use that same currency; MVP-1 performs no currency
  conversion. Canonical decimal values are at most 256 ASCII bytes, 127 integer
  digits, and 128 fractional digits; Task Domain and Policy share those exact
  bounds.
- Evaluation order is fixed: input validity; project/snapshot binding; runtime
  admission; role/action; state; protected routing; requested capability;
  current provider capability; network/deployment/cost; risk/approval; writer
  lease/fencing; resource budgets; allow.
- The minimum approval floor is `R0 = none`, `R1 = policy`, `R2 = responsible
  user`, and `R3 = responsible user plus independent security and architecture
  checks`. A Task Spec may raise but never lower that floor.
- Only the Implementer may hold a product-code writer lease. The Integrator may
  perform an authorized non-conflicting metadata merge but may neither write
  product code nor resolve a conflict; conflict repair becomes a new
  Implementer task.
- Project Registry and Workspace Git produce snapshot-bound physical
  local-ref identity digests. Policy uses those identities for primary-branch
  classification, preventing case-insensitive filesystem aliases without
  collapsing distinct case-sensitive refs.
- Every authority fact binds the exact project, snapshot, task, revision, Task
  Spec hash, and relevant action subject. Provider capability facts additionally
  bind provider identity, version, executable digest, runtime/schema identity,
  and freshness.
- An external network target without an immutable allowlist binding is denied.
  An authorized deployment decision is only an intent to invoke a future
  adapter. Unknown or newly introduced external cost requires responsible-user
  authority.
- Non-`ACTIVE` runtime admission rejects normal mutation. `DRAINING` permits
  bounded stop, reconciliation, and writer-release actions; `CANARY` permits
  only guardian health work; stopped or reconciliation-required modes permit
  only their exact recovery actions. A normal Runtime Supervisor may reconcile
  a typed resolved effect, proven holder death, or replaced leadership only to
  `STOPPED`; it cannot restore global `ACTIVE`. Guardian-only restoration to
  `ACTIVE` requires exact owner-produced durable saga, database, and boot-state
  evidence whose release, manifest, slot, and epoch agree.
- Policy may consume typed approval, nonce, lease, fencing, capability, and
  resource facts, but it does not authenticate, persist, refresh, claim, or
  consume them. Resource evidence is a fixed-producer Task Ledger receipt whose
  complete projection must equal a head obtained from an independent current
  owner lookup; caller-owned owner/producer/freshness fields are not accepted.
  Owner modules must authenticate and claim the facts atomically before a real
  effect.
- Memory and upgrade candidates are non-authoritative inputs. The first A/B
  activation path permits no database schema migration.
- Policy is pure Rust and performs no filesystem, database, process, clock,
  environment, network, credential, provider, or product-repository I/O.

### Project Registry owner boundary

- Project Registry owns the complete registered-project aggregate: canonical
  root, physical root identity, repository identity, filesystem/file identity,
  project class, lifecycle, accepted observation, pending drift, Registry
  revision, immutable snapshots, and reconciliation transitions.
- `lattice-contracts` 1.2 owns only shared immutable representations for canonical
  Project ID, closed project class/lifecycle, fully qualified local
  `refs/heads/*` physical Git-ref identity, fixed Registry producer/version,
  and task-agnostic Registry authority receipt/full-head values. Valid
  uppercase branch names remain allowed while an explicit pseudo-ref denylist
  rejects revision aliases. Shared representation does not transfer Registry
  state ownership.
- Registration issues revision/snapshot 1. Exact resolve of the same immutable
  observation reuses its current head. Suspend, drift observation, and
  successful move/identity/reactivation reconciliation advance the non-wrapping
  Registry revision and issue a new immutable snapshot.
- Command IDs, canonical-root text, and primary-ref text must already be NFC;
  hidden normalization cannot alter a request or hash subject.
- Duplicate project ID, accepted identity, or pending reserved identity during
  registration/reconciliation returns `Denied` without mutation. Alias path
  text cannot create a second project when its physical identity matches an
  accepted or reserved registration.
- The first non-colliding pending observation reserves its physical identities
  for its owning project until exact reconciliation. Another project cannot
  front-run that reservation.
- If an `ACTIVE` project's authoritative observation collides with another
  project's accepted or pending identity, retaining the old active authority
  is unsafe. Registry returns the distinct terminal outcome `Blocked`, rotates
  revision/snapshot, transitions the observed project to `SUSPENDED`, clears
  its colliding pending observation, and leaves the other project's
  reservation unchanged.
- Moved, replaced, suspended, stale, ambiguous, or cross-project identity
  evidence cannot issue active current authority. Project class is immutable
  after registration.
- Each Registry command, including an exact read-only observation, binds
  command ID, canonical request digest, before/after authority heads, terminal
  outcome, and result digest. Observe, suspend, and reconcile additionally
  bind the expected full authority head; register has no prior head. Same
  command/same request replays identically; same command/different request
  rejects. Registry 1.1's `result_digest` is the terminal semantic
  command-result commitment; Registry 1.1 has no separate terminal-receipt or
  record-set hash subject.
- TASK-012 accepts immutable fake `RepositoryObservation` inputs and issues
  only fake receipts. It performs no live path/Git inspection and makes no
  claim about PostgreSQL durability, restart, Windows file IDs, junctions, or
  loose/packed Git-ref representation.
- TASK-022 keeps Project Registry 1.2 pure and I/O-free while adding one
  runtime-aware vacant/plan/apply/export/verify boundary shared by Fake and
  PostgreSQL. Planning consumes only verified retained state; apply rechecks
  the complete base checkpoint; exported observations, projects, commands,
  reservations, and checkpoints remain untrusted. `RegistryCheckpoint::from_retained`
  reconstructs a separately read checkpoint value without claiming it is
  current. Plain `verify_untrusted_registry_snapshot` proves only internal
  self-consistency; a durable adapter must use
  `verify_untrusted_registry_snapshot_against_checkpoint` so a coherent older
  prefix cannot hide a removed denial or exact-observation tail.
- The immutable global Registry checkpoint binds runtime/version, one
  non-negative, non-wrapping signed-BIGINT-compatible catalog command
  high-water, every current project projection, every accepted and pending
  identity reservation, the complete first-seen semantic command history,
  deterministic counts, retained-byte accounting, logical state, and its
  canonical digest. The vacant high-water is `0`; first-seen command records
  are the strict sequence `1..N`. Every first-seen terminal command, including
  `Denied`, `Blocked`, and an exact observation that changes no project,
  advances the global checkpoint exactly once. Exact same-request replay does
  not advance it. Only a legal lifecycle mutation advances the target
  project's separate Registry revision and authority snapshot.
- Registry 1.2 uses one acyclic canonical commitment order. First build the
  checkpoint command core from only ordinal, complete typed request, and the
  complete semantic `RegistryCommandReceipt`; it excludes base/result
  checkpoint references, record-set/count/retained-byte fields, and all
  adapter evidence. Next build the domain logical-retained-state canonical
  bytes and result checkpoint. Then the record-set binds that command core
  plus base/result checkpoint references, any new immutable observation, an
  optional current-project replacement, and exact ordered reservation
  deletes/inserts. Only afterward may Postgres Store build the transaction
  digest and, last, the persistence receipt. Checkpoint references are
  verified as a strict chain but are never checkpoint inputs, and a physical
  command-row projection cannot replace a domain projection.
- `lattice.project-registry.logical-retained-state` schema version `1` canonicalizes exactly
  `schema_version`, `runtime`, `observations`, `projects`, `commands`, and
  `reservations`. Complete digest-keyed observations are sorted by digest and
  counted once even when multiply referenced; current projects are sorted by
  Project ID and reference observations by digest; checkpoint command cores
  are sorted by strict ordinal; reservations are sorted by identity dimension,
  identity digest, status, and Project ID. Optional fields are explicit
  canonical `null`, text is already NFC and counted as encoded UTF-8, and
  unsigned/count values are canonical decimal strings. Retained bytes equal
  only `canonicalize(logical_state).len()`: no hash frame, counts, retained-byte
  field, checkpoint references/digests, record-set fields/digests, SQL row
  overhead, database/schema fields, transaction digest, or persistence receipt
  participates. The exact vacant Live logical state is
  `{"commands":[],"observations":[],"projects":[],"reservations":[],"runtime":"LIVE","schema_version":"1"}`,
  which is 103 bytes. At high-water/counts zero, the frozen vacant checkpoint
  digest is `22ad9599c05ab384e720b8f1d91bdfbe1262360850602aa0f6b5fd79c1797f4f`
  for Fake and `5bb1f9d9adf7228bef7ea45cc029e79d71761c4dfed3abe086634917db38c173`
  for Live.
- Registry 1.2 retains no more than 4,096 current projects, 65,536 first-seen
  terminal commands, 67,108,864 bytes (64 MiB) of Registry-owned retained
  logical-state canonical bytes, or 131,072 UTF-8 bytes (128 KiB) in one already-NFC
  canonical root. Exact replay and changed-command-ID classification precede
  capacity checks; an over-limit first-seen request fails closed before
  mutation, checkpoint advance, or history truncation. TASK-022 adds no
  compaction or deletion policy.
- `FakeProjectRegistry` uses the same planner, apply, checkpoint, and verifier
  semantics. Registry 1.1 freezes only representative observation, request,
  authority-receipt, and command-result digest vectors; those and all existing
  TASK-012 behavior must remain byte-identical. Registry 1.2 introduces
  separate new checkpoint and record-set literal vectors rather than
  retroactively attributing either subject to 1.1.
- Postgres Store 1.4 persists the global plan without fabricating a
  project-scoped `StoreScope` or `ProjectSnapshotId`. Schema v4 adds exactly
  five Registry tables: `control.project_registry_state`,
  `control.project_registry_observations`,
  `control.project_registry_projects`,
  `control.project_registry_commands`, and
  `control.project_registry_identity_reservations`. It adds exactly nine
  fixed `project_registry_*_v1` runtime functions for prepare; reads of state,
  observations, projects, commands, and reservations; command/observation
  staging; project/reservation staging; and final checkpoint publication.
- `PostgresProjectRegistry` loads and verifies the complete global state, asks
  the pure Registry to reconstruct the retained checkpoint and verify the
  snapshot against that independently read singleton, then plans and commits
  the command, optional immutable observation, optional project/reservation
  replacements, final checkpoint, and distinct Registry persistence evidence
  in the normative acyclic order within one bounded `SERIALIZABLE`
  transaction. Exact replay precedes mutable admission. Restart, concurrent
  same/cross-project commands, commit-response loss, corrupt or partial rows,
  coherent-prefix rollback, checkpoint/receipt substitution, and schema/
  manifest drift must converge or fail closed without returning false
  Registry authority.
- Policy 2.3 consumes a Registry receipt plus a full head obtained through an
  independent current Registry-owner lookup and adds the Task Spec binding.
  It compares every receipt security field; `receipt.head()` alone is only a
  structural projection and cannot prove currentness. Registry never creates
  task IDs or Task Spec hashes, and Policy never imports Registry
  implementation state. Authenticated, serialized current-head lookup remains
  a future Orchestrator/PostgreSQL boundary.
- Future Orchestrator composes Task Spec, Registry receipt, Workspace-Git
  merge evidence, and Scope Check receipt. The Scope Check receipt must bind
  exact Task Spec/project/snapshot/Registry receipt, commit/head/diff,
  rule-set/report digests, producer/version, and observation revision before a
  live merge can be considered.

### Codex implementation lane

- The topology, once approved, uses one component as the owner of the writable
  Codex app-server process and native thread.
- Before use, the adapter verifies the exact executable identity/version/digest,
  a protocol schema generated by that exact binary and bound by hash, and
  explicit feature probes for every required method/notification. It does not
  assume complete automatic capability negotiation.
- Codex runs with a dedicated LATTICE-owned `CODEX_HOME`; the adapter verifies
  `initialize.codexHome`. The user's normal Codex home requires separate
  approval.
- It initializes the app-server, starts/resumes/forks only through the approved
  thread policy, streams normalized events, records `turn/completed`, and
  supports interruption.
- Codex permission requests are evaluated by LATTICE policy. Codex/OpenClaw
  approval mechanisms are defense in depth, not the source of authority.
- Worktree confinement is enforced by LATTICE RPC allowlists, a fixed
  permission profile, independently verified OS/process containment, and exact
  post-run Git/path Scope Check; it is not assumed from app-server.
- Token usage is recorded when emitted. Monetary cost is separately derived
  from a pinned pricing/account model or remains `unknown`.
- A second process may not resume or write the same native thread.

### Graphify knowledge lane

- Graphify reads an immutable repository snapshot or exact commit through a
  read-only source boundary and writes only to a separate LATTICE-owned
  artifact staging directory.
- The first live slice is code-only and local; semantic document/media passes
  are separate capabilities. `graphify install`, hook/skill installation,
  live PostgreSQL introspection, semantic backends, and optional external
  integrations are forbidden.
- Each graph snapshot records project identity, commit/tree hash, Graphify and
  adapter versions, artifact digest, build status, and capability hash.
- Graph files are derived and rebuildable for a pinned input/tool/config tuple.
  Byte-for-byte reproducibility is tested by LATTICE rather than assumed.
  Extracted, inferred, and ambiguous edges retain their labels.
- Inferred/ambiguous edges cannot independently authorize scope, dependencies,
  or code changes.
- If the exact version cannot place output outside the source root, preflight
  rejects it.
- TASK-033 pins official package `graphifyy==0.9.33`, tag/commit
  `v0.9.33`/`4e7e6b1f7e0df10ed07d5f28f9189bbde42940f1`, Apache-2.0, and wheel
  SHA-256 `c32b5792c783a6e66b1100b35bc65df3538e3f69b9df45fb098c9634c1b8eb01`.
  Runtime configuration binds a complete 2,184-file dependency payload plus
  the reviewed WSL/Python/bubblewrap execution boundary; it never follows
  `latest` automatically.
- The only permitted production invocation shape is equivalent to
  `graphify extract <immutable-snapshot> --code-only --no-cluster
  --max-workers 1 --out <owned-staging>` inside fixed `wsl.exe --exec` and
  bubblewrap user/mount/network namespaces. Runtime/source are read-only, only
  staging is writable, nested user namespaces are disabled, and unbound host
  paths/network are unavailable. LATTICE clears provider/backend environment,
  sets `GRAPHIFY_QUERY_LOG_DISABLE=1`, and rejects every install, hook, query,
  watch, global, live-PostgreSQL, semantic-backend, or in-source output path.
- LATTICE materializes the snapshot from tracked Git objects, records the exact
  commit/tree plus a sorted path/content-digest manifest, and excludes
  untracked or secret material before Graphify starts. Every returned
  `source_file` must resolve to that manifest.
- Graphify success is accepted only when the child exits successfully and a
  complete strict `graph.json` validates. Timeout, malformed JSON, unknown
  provenance, missing/partial output, or changed source binding rejects the
  run before durable graph-memory mutation.

### Hermes research lane

- Hermes uses a dedicated `HERMES_HOME`/profile for state separation plus
  independently enforced whole-process OS containment, read-only product input,
  a separate candidate-output directory, and no Git/database credentials. A
  profile alone is not a security boundary.
- It receives a bounded research/reflection request. It may return arbitrary
  output; LATTICE accepts only a versioned, schema-valid candidate envelope
  with required provenance and rejects or quarantines everything else.
- It cannot receive a product writer lease, general database credentials,
  promotion authority, or a writable Codex runtime/thread.
- Its memories, skills, summaries, and self-improvement outputs enter LATTICE
  only as untrusted candidates.
- Hermes memory/skill approval settings and guard-agent settings are defense in
  depth only. Unknown capabilities, malformed output, or an attempted product
  mutation blocks the lane.

### Codebase Memory

- Memory is project-isolated and snapshot-bound.
- Records distinguish `FACT`, `OBSERVATION`, `INFERENCE`, `DECISION`,
  `FAILURE`, and `PREFERENCE`, plus `CANDIDATE`, `QUARANTINED`, `ACCEPTED`,
  `REJECTED`, `SUPERSEDED`, and `EXPIRED` review states.
- Every accepted record points to one or more immutable source references and
  records creator, timestamps, content hash, schema/model/tool versions,
  reviewer, and supersession chain.
- Retrieval records query hash, project, snapshot, algorithm/version, returned
  IDs, and safety decisions.
- PostgreSQL full-text search is the zero-extension baseline to benchmark, not
  presumed sufficient. Traditional Chinese, mixed Chinese/English, Rust
  symbols/paths, error codes, exact filenames, and correct no-answer behavior
  are measured before choosing trigram/token-table/vector alternatives.
- Memory can inform planning but cannot alter policy, approvals, leases, scope,
  project identity, or release activation.
- `PREFERENCE` becomes accepted only from authenticated user evidence;
  model-inferred preferences remain `INFERENCE/CANDIDATE`.
- TASK-033 stores only normalized structural `OBSERVATION/CANDIDATE` records
  from the exact graph snapshot. It stores identifiers, labels, relation,
  confidence/provenance, source location and digests, but no raw source text,
  credentials, untracked content, or inferred authority.
- The first retrieval algorithm is LATTICE-owned, bounded, versioned, and
  deterministic: exact identifier/path/token matches outrank partial matches;
  ties use stable record identity. Each query binds exact project and commit,
  records query/algorithm/result digests and ordered record IDs, and returns no
  cross-snapshot result.
- Durable insertion, changed-source invalidation, retrieval audit, and status
  replay occur only through fixed PostgreSQL functions in the single LATTICE
  database. Rejection before final persistence has zero memory side effects.

### Approval authority

- Approval Verifier is the sole semantic owner of complete typed approval
  subject canonicalization, challenge/proof verification, nonce binding,
  availability/current-head, exact retry, replay, and claim preconditions.
- Contracts carries one complete neutral typed subject graph. Policy compares
  the exact subject in an owner receipt; an opaque caller digest or verification
  Boolean is never approval evidence.
- Challenges bind requester/proposer, approver, authority/trust lane,
  channel/session, nonce commitment, issue/expiry, runtime, complete
  project/task/spec subject, and authenticator/proof/evidence digests.
- Valid time is `issued_at <= observed_at < expires_at`. Policy reads no clock;
  an independent owner lookup returns a head only while the receipt is
  verified, unclaimed, unrevoked, and unexpired.
- Revocation applies only to a verified available normal approval or a
  verified protected approval pending Guardian claim. It requires the exact
  current owner head, a canonical observation inside the same validity
  interval, the original approving actor as revoker, and a non-zero revocation
  evidence digest. It advances the verifier revision to a terminal typed
  `REVOKED` state, removes current-head availability, and participates in exact
  retry, terminal receipt chaining, raw replay, and trusted checkpoints.
- The fake verifies only deterministic revocation binding. A live normal
  revocation must authenticate the original approver through the OS trust
  adapter; a live protected revocation must authenticate the original
  Guardian approver through the Guardian trust-root adapter. No other revoker
  or administrative override exists in Approval Verifier 1.0; adding one
  requires a versioned contract and ADR amendment.
- Responsible-user/OS-authenticated and protected-guardian/Guardian-trust-root
  are the only verified authority pairs. A normal gateway/model/candidate/
  daemon identity cannot become protected authority.
- Normal nonce claim occurs in the future transaction performing or claiming
  the approved transition/effect. Protected-release nonce claim occurs only in
  Guardian `claim_activation`, atomically with `ACTIVATION_CLAIMED` and
  `DRAINING`.
- Approval Verifier cannot establish independent review acceptance. Until
  Review Runtime supplies its own owner receipt/current head, R3 and every
  independent-review-required allow path fail closed.

### Independent review lane

- Review runs through a dedicated read-only port and never reuses the
  Implementer's process/thread, writer lease, or mutation capabilities.
- Reviewer set, evidence inputs, and acceptance hash are frozen in the Task
  Packet.
- The same model/runtime that implemented a change cannot be the sole
  acceptance authority. Protected/high-risk changes retain responsible-human
  approval.

### Controlled self-improvement and self-upgrade

- Outcomes can generate an improvement proposal linked to failures, metrics,
  user corrections, and affected contracts.
- The proposal becomes a normal Task Packet and follows the same
  implementation, verification, review, and approval gates as any product
  change.
- A candidate release is built in an inactive immutable slot and carries source
  commit/tree, lockfile, binary, migration, policy, capability, and evidence
  hashes.
- A separate guardian shadow-runs a candidate with a real read-only database
  role, drains the old daemon, runs a recoverable activation saga, monitors a
  health window, and rolls back to a prior compatible slot.
- The exact release approval binds actor/authority/channel/session, release
  revision, Task Spec hash, manifest/source/binary/migration hashes,
  schema/policy/capability deltas, target slot/epoch, one-use nonce, issue time,
  and expiry. The guardian verifies it through an independent trust root.
  Approval Verifier owns cryptographic/identity validation, while one
  guardian-only `claim_activation` transaction uniquely consumes the nonce,
  binds receipt/subject hashes, appends `ACTIVATION_CLAIMED`, and changes
  runtime admission to `DRAINING`.
- During handoff, the guardian is the sole writer to dedicated release/daemon
  epoch procedures; it cannot write task, memory, or project data.
- The durable saga records intent/outcome for:
  `STAGED -> PROMOTION_APPROVED -> ACTIVATION_CLAIMED
  -> OLD_DAEMON_DRAINED -> SLOT_POINTER_WRITTEN -> CANDIDATE_STARTED
  -> EPOCH_ACTIVATED -> HEALTH_WINDOW_PASSED -> ACTIVATION_FINALIZED`,
  with explicit rollback and `RECONCILIATION_REQUIRED` states.
- `DRAINING` blocks new tasks, leases, effect claims, and user-project side
  effects. `OLD_DAEMON_DRAINED` requires zero valid writer leases, zero
  claimed/running/unknown effects, reconciliation of every effect, and
  termination evidence for all writable Codex process trees.
- The local boot pointer is a checksum-bound projection containing activation
  ID, manifest hash, slot, and expected epoch. PostgreSQL wins during restart
  reconciliation; ambiguous evidence starts no writer.
- After epoch activation, the candidate must append/replay a guardian-defined
  canary on a reserved system stream through the normal epoch-checked path.
  Admission remains `CANARY`, so every other daemon mutation/effect is denied.
  The canary cannot touch a user project.
- Only `ACTIVATION_FINALIZED` changes admission to `ACTIVE`. Rollback starts
  the prior binary at a still-higher epoch in `CANARY` and sets `ACTIVE` only
  after its rollback canary/finalization passes. Ambiguity sets
  `RECONCILIATION_REQUIRED` and admits no daemon writes.
- The active binary is never overwritten in place.
- Rollback starts the prior binary as a new instance at a higher epoch; epochs
  never decrement. The first A/B MVP permits no schema migration.
- The LATTICE core cannot self-approve policy, constitution, supervisor,
  credential, network, database compatibility, or protected-capability changes.

## User Stories Or System Scenarios

1. As the owner, I can register any local Git project without editing LATTICE
   policy source.
2. As the owner, I can ask OpenClaw for a plan and see exact scope, risk,
   capabilities, cost limits, and acceptance evidence before approval.
3. As the owner, I can stop an active task and verify that the writer lease and
   Codex turn were terminated before review.
4. As an auditor, I can replay a task from PostgreSQL events and detect a
   changed, missing, reordered, or duplicate command/event.
5. As a developer, I can use Graphify evidence without treating inferred
   relationships as source facts.
6. As a learner, LATTICE can use Hermes to propose reusable lessons without
   letting Hermes write code or promote memory.
7. As the owner, I can see why a memory was retrieved, its source snapshot, and
   whether it is fact, observation, inference, or a rejected/stale candidate.
8. As the owner, I can allow LATTICE to prepare an improvement candidate while
   retaining the initial promotion decision and a rollback path.
9. As a recovery operator, I can stop or roll back the platform even when the
   normal gateway or new daemon is unhealthy.

## Goals

- A project-agnostic local development control plane.
- Rust-owned deterministic policy and orchestration.
- PostgreSQL-backed replayable truth and writer authority.
- One exclusive Codex code-writing lane.
- Useful read-only Graphify and Hermes evidence.
- Provenance-first Codebase Memory.
- A measurable, staged, reversible self-improvement lifecycle.
- Clear fake, static, live, human-accepted, and machine-enforced statuses.

## MVP Delivery Boundaries

The MVP names are cumulative product evidence levels, not substitutes for the
acceptance criteria below.

| MVP | Included behavior | Required evidence level | Excluded claim |
|---|---|---|---|
| MVP-0 — Rust foundation | buildable Rust workspace, inert CLI/bootstrap, versioned shared contracts and abstract ports | local machine checks plus independent code and architecture review | no durable store, provider execution, or autonomy |
| MVP-1 — Deliverable local alpha | one thin real chain from Codex App or OpenClaw through Rust/PostgreSQL to Codex modification, fixed test and local commit, followed by Graphify, Hermes and project-isolated Codebase Memory retrieval | exact component identity, durable intent/result and restart replay, changed-path/test/commit evidence, graph/reflection artifacts and a recorded memory query | no production hardening, public service, silent self-release, or claim from scripted/fake adapters |
| MVP-2 — Isolation and recovery | harden the MVP-1 chain with durable leases, exact scope enforcement, cancellation, reconciliation, component isolation and restart recovery | fault/cancel/restart/reconciliation tests, OS-boundary evidence, scope isolation, compatibility matrix and measured retrieval quality | no unconstrained agent, second writer/truth, public service, or self-release authority |
| MVP-3 — Guardian-protected autonomy | normal improvement Task Packets plus immutable A/B candidates, protected guardian activation, health/canary, reconciliation and rollback | fault injection, nonce/epoch/admission enforcement, complete-drain proof, power-loss recovery and rollback drill | no silent protected change or in-place self-overwrite |

MVP-0 is complete from TASK-008 and TASK-009 evidence. MVP-1 is current under
TASK-033 while TASK-032 official live remains `FAILED_DIAGNOSTIC`. TASK-010 through TASK-021 supplied the pure and PostgreSQL
foundations; TASK-022 through TASK-031 remain hardening backlog unless a gap
blocks the runnable path. TASK-032 first proves the official Codex app-server,
PostgreSQL restart replay, bounded workspace/test/Git commit, `latticed` MCP
entry, and compatibility wrapper. TASK-033 attaches real Graphify and Codebase
Memory; later MVP-1 nodes attach Hermes and then OpenClaw. MVP-1,
MVP-2, and MVP-3 remain incomplete until their direct exit evidence exists.

## Non-Goals

- A website-specific product, feature, or automation.
- Public cloud or multi-host consensus in the first implementation.
- Rewriting OpenClaw, Codex, Graphify, or Hermes in Rust.
- Letting two runtimes share writable ownership of a Codex thread/worktree.
- Treating external agent memory, a graph, a vector index, or a generated file
  as the control truth.
- Autonomous payment, account/credential changes, public publication,
  production deployment, permanent deletion, security-control disablement, or
  public network exposure.
- Automatic destructive database downgrade.
- Claiming hostile-process isolation without a separately verified OS sandbox.

## Constraints

- Rust-first control plane; thin TypeScript OpenClaw plugin and isolated Python
  adapters are permitted.
- PostgreSQL 17 is the initial supported local server target.
- No new package/component installation or database mutation during the
  planning gate.
- Existing dirty V1 changes must be preserved.
- Schema, event, Task Packet, IPC, and adapter contracts are versioned and
  unknown versions fail closed.
- Secrets never enter task/memory payloads or logs in raw form.
- Tests use disposable repositories, process homes, and a disposable database;
  no unrelated user project is used as a fixture.

## Module Impact

| Module | Proposed version | Impact |
|---|---:|---|
| lattice-cjson | 1.0 | Pure shared `lattice-cjson-1` byte/framing mechanism; caller modules retain hash-subject semantics |
| task-domain | 2.1 | Rust contract and V2 schema; V1 read compatibility; accounting currency is hash-bound |
| task-ledger | 2.1 | Pure Rust event/request/head/receipt/resource semantics plus shared vacant/plan/apply, retained-command replay, complete checkpoint, and conditional outbox-admission derivation; the fake remains visibly non-durable and all I/O stays in Postgres Store |
| policy-engine | 2.6 | Generic project/capability/upgrade policy; independent current Registry, Task Ledger, Writer Lease, and Approval Verifier full-head comparison; R3 denies pending Review Runtime authority |
| project-registry | 1.2 | Canonical repository identity and lifecycle plus one pure runtime-aware global vacant/plan/apply/export/verify boundary, separately reconstructed retained checkpoint, acyclic command-core/logical-bytes/checkpoint/record-set commitments, complete bounded history, and byte-identical Registry-1.1 Fake vectors |
| writer-lease | 1.0 | New lease/fencing/daemon-epoch domain owner |
| workspace-git | 2.0 | Worktree/Git/filesystem evidence only; consumes lease authority |
| scope-check | 1.1 | Language-neutral contract; mission remains detection-only |
| orchestrator-runtime | 2.2 | Preserve delivery ordering and add pure injected-port snapshot -> Graphify -> validate -> memory persist/retrieve ordering; no concrete adapter or transport dependency |
| latticed | 1.1 | Sole normal composition root; existing two zero-parameter MCP tools expose the preconfigured delivery plus graph-memory run and durable status without a third tool or new arguments |
| openclaw-adapter | 2.0 | Inert scaffold becomes a thin local IPC gateway |
| gateway-ipc | 1.1 | Bounded canonical six-action protocol, NFC-preserving encoder, truthful core-service errors, and deterministic fake loopback; live transport and OS authentication remain deferred |
| approval-verifier | 1.0 | Pure typed-subject/challenge/proof/nonce/time/current-head owner and deterministic fake; live trust/claim remains deferred |
| postgres-store | 1.4 | Preserved Store/Task-Ledger evidence plus the approved global schema-v4 Project Registry design; TASK-033 does not alter its `0005` reservation or current constitution |
| artifact-store | 1.0 | Pure project-scoped object/reference/provenance/quota/delete-claim semantic owner and deterministic fake; PostgreSQL/filesystem I/O remains deferred |
| codex-adapter | 1.1 | One writable app-server process/thread implementing the typed `DeliveryCodexPort`; generic `CodexPort` is not a second production path |
| review-runtime | 1.0 | New independent read-only review boundary |
| graphify-adapter | 1.1 | Exact Graphify v0.9.33 code-only child over a tracked immutable snapshot; verified private tmpfs copies, Landlock ABI 3, strict framed capture, and typed output/provenance validation |
| hermes-adapter | 1.0 | New contained research/candidate boundary |
| codebase-memory | 1.0 | Pure canonical structural observation, candidate-state, deterministic ranking and persistence-plan owner; PostgreSQL I/O remains in Postgres Store |
| self-upgrade-guardian | 1.0 | New A/B activation, health, and rollback boundary |
| lattice-core-bootstrap | 1.0 | Inert compile-time component manifest for the first Rust slice |
| lattice-cli | 1.0 | Read-only bootstrap inspection/recovery command; no runtime authority |
| lattice-contracts | 1.11 | Preserve delivery values and add immutable snapshot-manifest, Graphify analysis, graph-memory record/query/status and terminal receipt representations without I/O or authority |
| lattice-ports | 1.7 | Preserve delivery ports and add exact snapshot, typed Graphify analysis and Codebase Memory repository ports; generic GraphifyPort remains frozen outside production composition |

The user approved this module direction, the local bootstrap slice, and
continued local work on 2026-07-29. TASK-010 adds the pure technical
`lattice-cjson` module to avoid placing serialization/hashing in
`lattice-contracts` or coupling Task Ledger to Task Domain. Only modules with
an active constitution and a current ticket may be implemented; technical
packaging modules do not activate functional provider modules.

## Data, Privacy, And Security

- PostgreSQL roles follow least privilege. External tools do not receive the
  service writer role.
- Raw credentials, tokens, authorization headers, environment dumps, and
  unredacted prompts containing secrets are forbidden durable payloads.
- External results retain source component, binary/tool version, adapter
  version, task/spec hash, correlation ID, external run ID, sequence,
  timestamp, payload hash, and capability hash.
- Untrusted output is parsed against an exact schema and never promoted because
  it resembles an instruction or approval.
- Project identifiers and canonical roots isolate retrieval and execution.
- Audit evidence covers allow and deny decisions, cancellation, reconciliation,
  memory promotion/rejection, candidate activation, and rollback.
- Content-addressed artifacts may live outside PostgreSQL, but the database
  records their digest, provenance, owner, location class, and retention state.
- Artifact bytes live only in a LATTICE-owned artifact root. Writes use a
  temporary file, digest verification, durable flush, and atomic rename; safe
  cleanup follows PostgreSQL reference/retention evidence and never scans a
  product root as disposable output.

## Compatibility And Migration

- SPEC-001 and Task Packet V1 remain immutable historical contracts.
- The Node prototype is never a simultaneous writer with the Rust core.
- Rust characterization tests must reproduce approved V1 canonical hashes,
  transitions, approval subjects, event validation, policy denials, lease
  behavior, and Git evidence where contracts remain applicable.
- Existing file-ledger data, if any, is handled by a future read-only verifier
  and dry-run importer. Import is not assumed to exist.
- PostgreSQL migrations use checksums and explicit compatibility ranges.
- The first A/B MVP permits no schema migration. A later expansion-only
  protocol must prove active and candidate compatibility, use one migration
  lock/owner, record intent/outcome, and recover interruption. Destructive
  contract migrations remain separate human-approved work.
- OpenClaw, Codex, Graphify, and Hermes adapters pin exact tested compatibility
  only after authorized installation/preflight. The current official Codex
  manual labels app-server experimental, so its adapter remains replaceable
  and plans never imply a stable or live support guarantee.

## Error Cases And Edge Cases

- Unknown project, duplicate root, root identity drift, or repository moved.
- Task Packet V1/V2 confusion or canonical serialization mismatch.
- Reused command ID with different subject; stale expected sequence.
- PostgreSQL unavailable, serialization retry exhaustion, transaction outcome
  unknown, stale projection, or outbox claim lost.
- At-least-once external effect delivered twice without provider idempotency or
  reconciliation support.
- Lease counter overflow, duplicate active lease, expired heartbeat, daemon
  epoch mismatch, or a holder that cannot be proven dead.
- Codex binary/schema mismatch, second thread owner, app-server overload,
  malformed event, completion missing, timeout, cancellation failure, or
  permission request after lease revocation.
- Graphify snapshot built against a changed tree, unknown edge label, corrupt
  artifact, source-root output, install/hook attempt, or semantic/model/live-DB
  call in code-only mode.
- Artifact length/digest mismatch, oversized stream, cross-project reference,
  manifest/object/reference/task/project/staging quota exhaustion, stale
  generation/head, changed idempotency key, missing/corrupt bytes,
  unauthorized reference release, retained-object delete claim,
  retention/grace not elapsed, reference race, claim-token substitution,
  unknown delete outcome, denial-tail truncation, coherent metadata rollback,
  or a caller path used as a cleanup target.
- Hermes unavailable, unknown capability, unapproved memory/skill write,
  attempted product mutation, prompt injection, or malformed candidate.
- Cross-project memory retrieval, missing provenance, stale snapshot, memory
  poisoning, contradictory accepted facts, or supersession cycle.
- Candidate release changes policy/capabilities unexpectedly, active-slot
  corruption, health check disagreement, crash loop, rollback-incompatible
  schema, stale/replayed approval, partial saga step, epoch split brain, or
  guardian unavailable.
- Git conflict, out-of-scope write, ignored/untracked path, link/junction
  escape, hook/driver execution, or uncertain cleanup ownership.

## Acceptance Criteria

- [ ] AC-01: Active V2 product artifacts describe a general platform and
  contain no project-specific policy/action/example dependency; preserved V1
  constitutions are explicitly scoped as legacy blockers rather than V2 rules.
- [ ] AC-02: Rust Task Domain and Policy reproduce all retained V1
  characterization fixtures and reject unknown V2 contracts.
- [x] AC-03: PostgreSQL append atomically enforces command idempotency,
  expected sequence, predecessor/event hashes, event append, stream head, and
  outbox intent, and terminal receipt under a unique stream/command key.
- [x] AC-04: Replay produces the same projection after restart; corruption or
  projection/head disagreement fails closed.
- [ ] AC-05: Concurrent lease acquisition permits one Implementer, emits a
  monotonically increasing non-wrapping `BIGINT` token, rejects stale
  daemon-instance/epoch/token tuples on every daemon-authorized durable
  mutation, enforces runtime admission on mutation/effect claims, and recovers
  a suspect lease only with holder-death evidence.
- [ ] AC-06: Project registration/reconciliation, worktree creation,
  changed-path evidence, and Scope Check reject duplicate/drifted identities,
  outside-root paths, links, hook/driver behavior, conflicts, and cleanup
  ambiguity.
- [ ] AC-07: An exact-version live OpenClaw plugin can
  submit/plan/status/approve/reject/stop through authenticated OS-local typed
  IPC without direct database, Git, provider, credential, protected-release,
  or product-path access. TASK-017 may close only the pure/fake protocol
  portion; live plugin, transport, peer authentication, disconnect/restart,
  and compatibility evidence remain MVP-2.
- [ ] AC-08: Codex adapter verifies exact binary/digest, same-binary schema
  hash, explicit features, and dedicated `CODEX_HOME`; it maintains one
  writable process/thread owner, enforces the LATTICE worktree boundary,
  streams evidence, interrupts, and fails closed on incompatible behavior.
- [ ] AC-09: Graphify reads a digest-bound code-only source snapshot, writes
  only to LATTICE artifact staging, invokes no install/hooks/live DB/semantic
  backend, and preserves extracted/inferred/ambiguous provenance.
- [ ] AC-10: Hermes arbitrary output crosses an independently enforced
  read-only product boundary and becomes a candidate only after schema and
  provenance validation; product, Git/DB credential, promotion, and writable
  Codex capabilities remain unavailable.
- [ ] AC-11: Memory candidates require immutable sources and explicit review;
  cross-project, unapproved, poisoned, expired, or superseded records are not
  injected as trusted context.
- [ ] AC-12: Every retrieval records its query, project/snapshot, algorithm
  version, returned IDs, and safety decision.
- [ ] AC-13: An improvement proposal follows the same task, scope, test, review,
  and approval path as user-requested changes.
- [ ] AC-14: A candidate release is immutable, exact-subject approved,
  shadow-verified, advanced by the guardian through every durable saga state,
  atomically claimed with nonce consumption plus `DRAINING`, proven free of
  leases/effects/writable Codex children, epoch-activated in `CANARY`,
  write-canary checked before `ACTIVE`, monitored, and rolled
  back/reconciled after interruption without treating a file as truth.
- [ ] AC-15: The core cannot promote changes to policy, constitutions,
  supervisor, credentials, network exposure, destructive migrations, or other
  protected capabilities without responsible-human approval.
- [ ] AC-16: Stop prevents new effects, interrupts the active provider,
  reconciles unknown outcomes, and revokes the writer lease before review.
- [ ] AC-17: No verification fixture touches an unrelated user repository or
  stores a raw secret.
- [ ] AC-18: Focused tests, full local checks, independent code review,
  architecture review, synchronization, and CI/merge authorization remain
  separate evidence gates.
- [ ] AC-19: Artifact writes are content-addressed, digest-verified, atomic,
  retained by database references, and never use a product root as disposable
  storage.
- [ ] AC-20: Review uses a separate read-only runtime/thread with frozen
  evidence/acceptance hashes; an Implementer is never its sole acceptance
  authority.
- [ ] AC-21: `lattice-cjson-1` byte fixtures cover Unicode, ordering, numeric
  strings, timestamps, null/missing, algorithm domain separation, and distinct
  approved V1 compatibility fixtures.
  TASK-010 supplies the V2 Unicode/order/escape/null/domain fixtures, validated
  numeric/timestamp strings, immutable-field mutation matrix, strict
  leap-second denial, V1 transition characterization, and explicit V1/V2 hash
  path separation. AC-21 remains open because the V1 candidate hash has not
  been promoted into a separately approved compatibility manifest. AC-02 also
  remains open until Policy V2 and the retained V1 characterization set are
  complete.
- [ ] AC-22: Memory retrieval meets an approved benchmark for Traditional
  Chinese, mixed Chinese/English, Rust symbols/paths, error codes, filenames,
  and no-answer behavior before an indexing strategy is accepted.
- [x] AC-23: Versioned Rust adapter-boundary contracts reject empty identities,
  malformed SHA-256 references, and unknown contract versions before any port
  call. Verified by TASK-009 focused and full local tests on 2026-07-29.
- [x] AC-24: Gateway, product-code writer, knowledge, research, and control
  store ports depend only on the contracts crate and return lane-specific
  evidence types that cannot cross-label component/authority pairs; no
  concrete adapter depends on another adapter or on Orchestrator. Verified by
  TASK-009 tests, Cargo metadata inspection, and independent review on
  2026-07-29.
- [x] AC-25: Policy V2 consumes a complete immutable Task Spec and returns
  deterministic typed decisions that default unknown or missing inputs to
  deny; it enforces exact project/snapshot binding, role/action/state,
  risk/approval floors, requested/current provider capability, One Writer,
  runtime admission, Task-Ledger-bound single-currency resource budgets,
  memory non-authority, exact merge/recovery facts, and stage-specific
  protected activation/rollback separation without I/O.
- [x] AC-26: Project Registry 1.1 deterministically registers one immutable
  physical project identity; reserves accepted pending identities; rejects
  duplicate registration/reconciliation aliases without mutation; and
  defensively rotates an authoritative cross-project collision to a
  `Blocked`, non-active `SUSPENDED` head without stealing the other
  reservation. It preserves old receipts, requires NFC command/root/ref hash
  subjects, enforces exact idempotent commands, and supplies a task-agnostic
  fake authority receipt plus independent current-head lookup that Policy 2.3
  rejects when any full-head field is stale, substituted, non-active, or
  fake/live mismatched. Contracts 1.2 fixes producer/version, preserves valid
  uppercase branches, and treats receipt-derived heads only as structural
  projections. Verified by TASK-012 focused/full tests, dependency/I/O checks,
  governance validation, independent code/security/architecture review, and
  local combined integration on 2026-07-29.
- [x] AC-27: Task Ledger 2.0 deterministically binds complete task streams,
  separately hashes request/event/head/receipt/resource subjects, performs
  exact command retry before stale-head denial, rejects corrupt/unknown replay,
  derives resource projection only from verified events, and issues a
  fixed-producer fake resource receipt whose complete projection must equal an
  independent current owner head in Policy 2.4. TASK-013 satisfied only this
  pure/fake semantic criterion; AC-03 PostgreSQL transaction atomicity and the
  durable/restart portions of AC-04 remained open at TASK-013 closure and were
  closed by TASK-021. Verified by TASK-013
  Contracts/Ledger/Policy matrices, real fake-owner current-head composition,
  dependency/I/O/governance checks, independent code/security/architecture
  review, and local combined integration on 2026-07-29.
- [x] AC-28: Writer Lease 1.0 deterministically binds the complete
  project/snapshot/task/revision/spec/attempt/holder/worktree/process/daemon/
  epoch/fence identity; plans and verifies acquire, heartbeat, suspect,
  release, revoke, and reacquire through one public pure semantic core; uses
  positive signed-BIGINT-compatible non-wrapping/non-reused fences; preserves
  exact applied and denied command receipts in a predecessor-bound receipt
  chain; detects claimed denial-tail truncation; requires an independently
  retained project/high-water/tail/snapshot checkpoint for rollback-sensitive
  restore; and issues a fixed-producer fake authority receipt whose complete
  projection must equal an independent current owner head in Policy 2.5.
  TASK-014 may satisfy only this pure/fake semantic criterion. AC-05 remains
  open for PostgreSQL concurrency, database time, atomic checkpoint storage,
  restart, stale live connections, and same-transaction durable mutation
  fencing. Verified by TASK-014 Contracts/Writer/Policy matrices, strict raw
  replay and trusted-checkpoint regressions, dependency/I/O/governance checks,
  independent code/security/architecture review, and local combined
  integration on 2026-07-29.
- [x] AC-29: Approval Verifier 1.0 deterministically validates and
  domain-hashes one complete typed approval subject; permanently binds one
  nonce commitment to the exact requester, approver, authority/trust lane,
  channel, session, challenge, project/task/spec subject, issue time, expiry,
  runtime, and proof/evidence identities; preserves exact applied and denied
  command retry before stale/time evaluation; rejects changed command content,
  nonce rebinding, subject/identity substitution, self-approval, invalid time,
  normal/protected trust substitution, replay, and corrupt/rolled-back
  aggregate history; and issues a fixed
  `lattice-approval-verifier`/`1.0` fake authority receipt whose complete
  projection must equal an independently queried available owner head in
  Policy 2.6. Policy contains no caller approval or independent-review verdict
  Boolean; R3 fails closed pending Review Runtime owner evidence. TASK-015
  proves only pure/fake semantics. OS authentication, live cryptographic trust
  roots, PostgreSQL uniqueness/database time/durability/restart/atomic claim,
  OpenClaw approval IPC, Review Runtime, and Guardian activation remain open.
- [x] AC-30: Artifact Store 1.0 deterministically binds project-scoped
  `(project_id, sha256)` objects, positive generation/revision, complete
  immutable provenance references, exact byte length/digest, fixed bounds,
  atomic object/task/project/store quotas, typed initial/reference/read owner
  authority, exact retry, terminal reference release, current-head equality,
  replay, checkpoint,
  durable-delete-claim/unknown-outcome/reconciliation semantics, and safe sweep
  preconditions through a visibly non-durable in-memory fake. Cross-project
  deduplication/existence sharing, provider receipt authority, caller
  counts/retention Booleans/bare evidence digests, and public filesystem
  deletion are absent. TASK-016 may satisfy only this pure/fake criterion;
  PostgreSQL reference transactions/durability/restart and real filesystem
  stage/flush/rename/link/sweep evidence remain open under AC-19.
  TASK-016 completed this bounded pure/fake criterion on 2026-08-01 with 32
  Contracts tests, 97 Artifact Store tests, 322 locked full-workspace Rust
  tests, and 38 preserved Node characterization tests passing. Strict format,
  workspace Clippy, dependency, forbidden-I/O/provider/product, unrelated-
  website, raw-byte-containment, and diff checks pass. Independent final code/
  security and architecture reviews report `PASS` with zero P0 through P3
  findings, and local combined integration passes. PostgreSQL durability,
  restart, real filesystem containment/deletion, and live authority remain
  explicitly open under AC-19 and later tickets.
- [x] AC-31: Gateway IPC 1.1 implements wire protocol 1.0 and rejects
  non-canonical/non-NFC, normalization-expanding, duplicate-key, numeric,
  oversized, over-deep, over-node, unknown-field/action/version, malformed,
  Task-Spec-digest/binding-mismatched, protected-release, and recovery-role
  overreach inputs before a service call; carries only six action-specific
  requests; keeps server-derived fake peer context outside request bytes;
  returns typed bound replies/component-free core errors that distinguish routing, stop request,
  terminal/no-op, denial, and unknown outcome; and proves exact scoped command
  retry/substitution plus fault behavior through a zero-I/O in-memory fake.
  TASK-017 completed this bounded pure/fake criterion on 2026-08-01 with 36
  Contracts, 31 Gateway IPC, and 3 Ports tests (70 focused), 358 locked full-
  workspace Rust tests, and 41 Node tests passing. Strict format/Clippy,
  dependency and forbidden-I/O/product scans, 17 unique-ticket/one-current-
  marker governance checks, independent code/security and architecture
  reviews, and local integration all pass. AC-07 remains open for live
  OpenClaw transport, OS authentication, restart, and compatibility evidence.
- [x] AC-32: Postgres Store 1.0 defines a bounded typed physical transaction
  envelope and deterministic zero-I/O fake. It binds one canonical project/
  snapshot/closed-owner/aggregate scope, globally unique transaction ID,
  domain-command commitment, independently retained daemon authority and
  physical heads, record-set/next-state/domain-receipt commitments, and
  optional checkpoint/outbox commitments. It recomputes the complete request
  digest; exact retry returns the identical terminal receipt before mutable
  checks, changed ID reuse fails with zero mutation and no receipt disclosure,
  valid apply advances one checked revision atomically, stale head is a stable
  non-mutating terminal denial, and before/after-apply faults preserve explicit
  unknown-outcome reconciliation. Every TASK-018 receipt is fixed to
  `RuntimeKind::Fake` and `NonDurableFake`. No SQL, driver, connection,
  migration execution, domain legality, durable PostgreSQL evidence, provider,
  product, or protected-action surface exists. AC-03/04 remained open at
  TASK-018 closure and were closed by TASK-021; AC-05/19 and the MVP-1 exit
  gate remain open for later disposable-database tickets.
  TASK-018 completed this bounded pure/fake criterion on 2026-08-01 with 42
  Contracts, 5 Ports, and 14 Postgres Store package tests (61 focused), 380
  full-workspace Rust tests, and 44 Node tests passing. Strict format/Clippy,
  governance, dependency, migration-inactivity, forbidden-I/O/SQL/driver/
  provider/product/website scans, independent reviews, and local integration
  all pass. The receipt remains explicitly `NonDurableFake`.
- [x] AC-33: Postgres Store 1.1.5 uses exact-pinned `postgres` and SHA-256
  dependencies to expose an explicit, ordered migration manifest and a
  separately invoked migration runner plus runtime schema verifier. The
  unchanged `0001_bootstrap.sql` hash is retained as `SUPERSEDED` and never
  executed; one new transaction-control-free migration creates the owned
  database identity, exact migration ledger, generic physical-head/terminal-
  transaction foundation, and singleton runtime-admission foundation. The
  runner first verifies an exact non-default target database plus a
  pre-provisioned disposable-run sentinel, then verifies every manifest byte
  before SQL; it rejects directory discovery,
  unknown/missing/reordered/checksum-drifted history and pre-existing unowned
  schemas, obtains one transaction-scoped advisory lock, and applies missing
  executable entries atomically. Normal runtime startup verifies only and
  cannot migrate. Bootstrap admission is `STOPPED` with no daemon leader;
  TASK-019 exposes no normal path to self-promote `ACTIVE`. Runtime, migration,
  guardian, and read-only capability roles are permission-separated without the
  runner creating a login, password, database, or credential. Each externally
  provisioned fixed LOGIN principal has exactly one `ADMIN FALSE, INHERIT
  FALSE, SET TRUE` membership and must `SET ROLE` to its matching NOLOGIN
  capability. Across the cluster, `PUBLIC` has no database ACL, each LOGIN has
  exactly one non-grantable direct `CONNECT` grant from `lattice_migrator` on
  the exact target and no database ACL elsewhere, and `pg_parameter_acl` has no
  grant to `PUBLIC` or any of the eight fixed roles. Before `SET ROLE`, every
  LOGIN has no inherited capability, `CREATE`, `TEMPORARY`, or direct grant in
  any PostgreSQL ACL-bearing catalog. External non-system relations and columns
  have no ownership or ACL for `PUBLIC` or any fixed role, and every external
  non-system function denies effective execution to `PUBLIC` and all eight
  fixed roles. Every recorded `pg_default_acl` owner has zero `PUBLIC` grant,
  not merely owners in the fixed-role set. Cluster-wide `pg_shdepend`
  `deptype = 'o'`, cross-checked by the verifier's explicit current-database
  owner checks, proves that none of the four fixed LOGIN principals owns an
  object in any database or shared catalog.
  Before `SET ROLE`, an exact protected-function manifest denies all fixed
  LOGIN principals effective execution of `lo_creat(integer)`, `lo_create(oid)`,
  `lo_from_bytea(oid,bytea)`, both `lo_import` overloads, both four-argument
  `pg_logical_emit_message` overloads, all sixteen advisory-
  lock acquisition overloads, `pg_export_snapshot()`, `pg_current_xact_id()`,
  and `txid_current()`. Two concurrent sessions authenticated as the same fixed
  LOGIN also prove denial of `pg_cancel_backend(integer)` and
  `pg_terminate_backend(integer,bigint)`. Of the sixteen lock-acquisition
  overloads, only `pg_advisory_xact_lock(bigint)` is granted to
  `lattice_migrator`, and only after `SET ROLE`; no fixed LOGIN receives that
  grant directly. That single direct grant is non-grantable and originates from
  the protected function owner. The disposable harness proves these boundaries with one-time
  test-only SCRAM identities instead of superuser impersonation. Normal runtime
  has no direct table DML,
  DDL, admission/history/identity write, role escalation, or effective CREATE
  privilege in any non-system schema. To preserve concurrent-runner convergence,
  the migration
  runner uses one read-committed writable transaction under its transaction-
  scoped lock, commits, and only then invokes the same repeatable-read,
  read-only verifier used at runtime. A known successful commit followed by a
  verifier failure returns `PostApplyVerificationFailed`
  (`STORE_MIGRATION_COMMITTED_UNVERIFIED`); reconnecting with the identical
  manifest must converge to `AlreadyCurrent` and pass the verifier. A commit
  response whose outcome is unknown remains a separate failure. Every returned
  catalog proof therefore uses one consistent snapshot. The database identity
  is a deterministic, domain-separated SHA-256 custom UUIDv8 bound to the exact
  target and run marker. The verifier reads authoritative tables through
  `ONLY`, includes live column ACLs in the
  signature, requires the exact owned table-row and generated array `pg_type`
  allowlist, rejects shell or extra owned types, and rejects inheritance/
  partition, dropped-column, catalog/owner/constraint/grant/default-privilege
  drift even if history appears exact. The verifier also requires
  `max_prepared_transactions = 0`, so no prepared transaction can retain
  unaccounted write authority across the runtime boundary. `LISTEN`/`NOTIFY`
  is never authoritative state, admission, evidence, or an effect-delivery
  source; because `NOTIFY` is a SQL command rather than a function call, this
  criterion does not claim that function revocation prevents it. Live
  regressions prove both catalog drift rejection and the corresponding real-
  LOGIN pre-`SET ROLE` behavior. A disposable,
  owned, loopback-only PostgreSQL 17.10 cluster proves first apply, exact
  no-op retry, concurrent runners, rollback, history/checksum/role/settings
  denial, restart persistence, pre-create reparse-boundary checks, exact
  stopped-state proof, cleanup-before-PASS, and redacted failures. The
  existing zero-I/O fake remains unchanged and every Store receipt remains
  `NonDurableFake`; live `ControlStore`, domain records, durable receipts,
  daemon/Guardian activation, remote/TLS connections, and user/production
  database mutation remain outside TASK-019.
- [x] AC-34: Contracts 1.9 preserves Store contract v1 as fake-only and adds a
  v2 live/durable PostgreSQL receipt whose exact runtime, durability, database-
  identity commitment, schema version, manifest commitment, request, before/
  after heads, disposition, transaction digest, and receipt digest are bound.
  Ports 1.4 makes physical-head observation explicitly mutable for synchronous
  clients without exposing a driver. Postgres Store 1.2 upgrades an exact v1
  foundation to schema v2 by accepting only an exact migration-history prefix,
  executing only missing immutable entries, and atomically advancing history/
  compatibility. Its live adapter consumes a caller-supplied runtime client,
  reads no DSN/credential/environment, and performs prepare plus finalize in
  one bounded transaction. Exact replay/changed-ID classification precedes
  admission; a new mutation revalidates exact `ACTIVE` daemon instance, epoch,
  authority revision and digests plus the locked physical head in the same
  transaction. Applied head and durable terminal receipt commit together;
  stale head creates a durable non-mutating receipt; commit-response uncertainty
  returns no receipt and converges only through reconnect plus exact retry.
  Runtime retains zero direct SELECT/DML on physical/terminal tables and may
  execute only the exact schema-qualified, safe-search-path, dynamic-SQL-free
  Store function allowlist. Guardian, reader, `PUBLIC`, and pre-`SET ROLE`
  LOGINs cannot call it. A test-admin-only ACTIVE fixture is not a production
  activation API. Disposable PostgreSQL 17.10 tests prove fresh and v1-to-v2
  migration, concurrency, retry, restart, fault, ACL, project/snapshot/owner/
  aggregate isolation, and direct-table denial. TASK-020 adds no Ledger,
  Registry, Lease, Approval, Artifact, Guardian, provider, product, website,
  production credential/database, remote/TLS, publication, or deployment path;
  AC-03/04 remained open at TASK-020 closure and were closed by TASK-021;
  AC-05/19 remain open for their durable repositories and effects.
- [x] AC-35: Task Ledger 2.1 exposes one pure runtime-aware vacant/plan/apply/
  checkpoint boundary shared by Fake and PostgreSQL without changing existing
  request/event/head/receipt hashes. Its complete checkpoint binds identity,
  head, replay-derived resources, ordered events, every terminal command
  including denials, and exactly one admission for each appended
  `EFFECT_INTENT` with audit outcome `RECORDED`. Existing non-`RECORDED`
  effect-intent combinations remain append-compatible but derive no admission;
  exact retry changes no state and changed command reuse
  reveals no receipt. Postgres Store 1.3 advances only an exact fresh/v1/v2
  prefix to global schema v3, preserves all existing migration bytes and
  frozen Store-v2 receipt evidence, and exposes only three new Store plus five
  Task Ledger fixed functions while revoking runtime execution of the three
  historical v2 Store functions. `PostgresTaskLedger` uses the pure planner
  and one bounded `SERIALIZABLE` transaction to commit the command, optional
  event/outbox, head/projection/checkpoint, and applied physical Store receipt
  together. Restart rebuilds the same verified stream and typed appended or
  denied receipt; event/command/outbox/head/checkpoint/physical corruption
  fails closed. Domain `u64` values remain exact through constrained
  `numeric(20,0)`. Commit-response loss returns no receipt, poisons the old
  instance, and reconciles only with a new client plus exact retry. TASK-021
  performs no live resource observation, outbox claim/delivery, other domain
  repository, activation, provider/product, production, release, deployment,
  or unrelated website work. Completion of this criterion also closes AC-03
  and AC-04; it does not close AC-05 or AC-19. TASK-021 completed this
  criterion on 2026-08-02 with exact schema-v3 migration and Store-v2 receipt
  preservation, atomic command/event/outbox/checkpoint/physical persistence,
  restart replay, bounded timeouts, coherent manifest-drift rejection,
  current-transaction Store-terminal proof, outbox-linkage and wrong-scope
  corruption rejection. The marker-owned PostgreSQL 17.10 initial/restart
  harness, 432 Rust tests, 44 Node tests, strict format/Clippy, 109-dependency
  RustSec audit, independent code/security and architecture reviews, and local
  integration all pass.
- [x] AC-36: Project Registry 1.2 exposes one zero-I/O, runtime-aware vacant/
  plan/apply/export/verify boundary shared by Fake and PostgreSQL. Vacant
  checkpoint high-water is `0`; first-seen command records are exactly `1..N`.
  Every first-seen terminal `Denied`, `Blocked`, applied mutation, or exact
  no-project-change observation advances the global checkpoint exactly once;
  exact same-request replay advances neither, and changed command reuse
  discloses no receipt. Only legal lifecycle mutation advances the separate
  per-project Registry revision and immutable authority snapshot.
  `RegistryCheckpoint::from_retained` reconstructs a separately read retained
  checkpoint without asserting currentness. Plain
  `verify_untrusted_registry_snapshot` proves only internal self-consistency;
  durable current authority requires
  `verify_untrusted_registry_snapshot_against_checkpoint`, which also compares
  that singleton and rejects a coherent older prefix. The acyclic commitment
  order is checkpoint command core (ordinal, complete typed request, complete
  semantic `RegistryCommandReceipt`, with checkpoint/record-set/adapter fields
  excluded), logical retained-state canonical bytes, result checkpoint,
  record-set (command persistence core, optional inserted observation/project,
  ordered reservation deletes/inserts), transaction digest, then persistence
  receipt. The logical-state object contains exactly `schema_version`,
  `runtime`, `observations`, `projects`, `commands`, and `reservations`:
  observations are complete, digest-keyed, sorted, and counted once; projects
  are Project-ID sorted and reference observation digests; command cores are
  strict-ordinal sorted; reservations are sorted by dimension/digest/status/
  Project ID; optional values are canonical `null`; text is NFC encoded UTF-8;
  and unsigned/count values are canonical decimal strings. The exact byte
  count is `canonicalize(logical_state).len()` and excludes hash framing,
  counts/the byte field, checkpoint references/digests, record-set fields/
  digests, SQL overhead, and all database/schema/transaction/persistence
  evidence. The exact vacant Live logical state is
  `{"commands":[],"observations":[],"projects":[],"reservations":[],"runtime":"LIVE","schema_version":"1"}`
  at 103 bytes; its frozen zero-count checkpoint digests are
  `22ad9599c05ab384e720b8f1d91bdfbe1262360850602aa0f6b5fd79c1797f4f`
  for Fake and `5bb1f9d9adf7228bef7ea45cc029e79d71761c4dfed3abe086634917db38c173`
  for Live. Registry 1.1 golden vectors cover only observation, request,
  authority-receipt, and command-result digests, with `result_digest` as the
  terminal semantic commitment; checkpoint and record-set are new Registry
  1.2 vectors. Capacity fails closed before mutation or truncation at 4,096
  current projects, 65,536 first-seen terminal commands, 67,108,864 logical-
  state canonical bytes (64 MiB), or 131,072 UTF-8 bytes (128 KiB) for one
  already-NFC canonical root; exact replay/changed-ID classification occurs
  first. Postgres Store 1.4 preserves migrations `0001` through `0004`
  byte-for-byte and advances only an exact accepted prefix to schema v4 with
  exactly five fixed-column Registry tables (`project_registry_state`,
  `project_registry_observations`, `project_registry_projects`,
  `project_registry_commands`, and `project_registry_identity_reservations`)
  and exactly nine fixed `project_registry_*_v1` functions for prepare, five
  complete projection reads, command/observation staging,
  project/reservation staging, and final checkpoint publication. One bounded
  `SERIALIZABLE` global transaction verifies the retained checkpoint, obtains
  the pure plan, and atomically commits the command, optional observation,
  optional project/reservation replacements, result checkpoint, and distinct
  Registry persistence receipt without constructing a project-scoped
  `StoreScope`, `ProjectSnapshotId`, or `StoreTransactionReceipt`. Fresh and
  exact v1/v2/v3/v4 migration, restart, same/cross-project concurrency,
  collision/reservation serialization, commit-ack loss, timeout, ACL/profile,
  current-transaction staging, and command/project/reservation/checkpoint/
  persistence corruption matrices must pass. Historical Store-v2 receipts,
  Task Ledger commands/receipts/checkpoints, and new Ledger appends remain
  byte-identical and available through the schema-v4 successor surface. This
  criterion closes only TASK-022 durable Registry evidence: AC-26 remains the
  completed pure Registry criterion, while AC-06 and MVP-1 through MVP-3 remain
  open.
- [ ] AC-37: TASK-032 provides one pure Rust delivery coordinator that can
  reach effects only through typed ports and orders durable intent before the
  Codex effect, bounded changed-path inspection and fixed test before Git
  commit, and durable outcome/receipt after the terminal effect. The sole
  normal `latticed` composition root is implemented by the existing
  `apps/lattice-runtime` package and exposes exactly the zero-parameter MCP
  tools `lattice_delivery_run` and `lattice_delivery_status`; their schemas
  accept no shell, SQL, path, credential, provider, or arbitrary task input.
  The `lattice-runtime` compatibility wrapper reaches the identical
  composition rather than a second orchestrator or truth source. Scripted
  protocol acceptance remains labeled as such; completion additionally
  requires an official Codex app-server turn, isolated changed-path/fixed-test/
  local-commit evidence, PostgreSQL restart replay from a separate status
  invocation, and fail-closed timeout/protocol/test/Git/database ambiguity.
- [ ] AC-38: TASK-033 pins and actually invokes Graphify v0.9.33 against a
  LATTICE-materialized exact tracked commit snapshot, strictly validates and
  canonicalizes complete code-only graph output, stores project/commit/tree/
  manifest/tool/config/content-digest-bound structural candidates plus a
  deterministic exact-snapshot retrieval audit in a separately versioned,
  independently hashed same-database Memory extension profile, and replays the
  same typed analysis/memory status after process and database restart. The
  extension must not alter global Store v3 state or the Registry-reserved
  global `0005`/schema-v4 profile, and its PostgreSQL implementation requires
  an explicitly approved versioned owning-module amendment first. Changed
  source invalidates the old current snapshot; untracked and
  secret files never enter the snapshot; timeout, malformed/partial output,
  unknown source provenance, or persistence ambiguity fails closed with zero
  false success. `latticed` retains exactly its two zero-parameter MCP tools
  and accepts no new caller-controlled query/path/shell/SQL/credential input.

## Verification Plan

| Criterion | Verification method | Expected evidence |
|---|---|---|
| AC-01 | repository text/rule scan | zero active project-specific policy references |
| AC-02, AC-21 | manually approved Rust golden/characterization tests | same retained V1 results plus frozen V2 canonical bytes |
| AC-03 to AC-05 | disposable PostgreSQL integration/concurrency tests | atomic append, replay, lease, overflow, restart evidence |
| AC-06 | disposable registry/Git repositories and platform path fixtures | identity lifecycle and exact deny/pass manifests; no external path touched |
| AC-07 | TASK-017 fake contract baseline then MVP-2 exact-version live OpenClaw/transport preflight | fake schema cannot substitute for live plugin, peer-auth, disconnect, restart, or compatibility evidence |
| AC-08 | fake server then exact-version live preflight | identity/digest, same-binary schema hash, feature probes, dedicated home, event/stop/scope evidence |
| AC-09 | immutable read-only source fixture and separate artifact root | no source writes/install/hooks; provenance-labeled derived output |
| AC-10 | OS sandbox/capability/adversarial tests | only valid envelopes accepted; mutation/credential attempts denied |
| AC-11 to AC-12, AC-22 | memory poisoning, isolation, provenance, multilingual/symbol retrieval tests | conservative accepted-only results, measured threshold, and audit |
| AC-13 to AC-16 | deterministic end-to-end, approval replay, power-loss, epoch, and A/B rollback drills | no self-approval, safe stop, recoverable saga, write canary, rollback |
| AC-17 | test-root and secret-leak assertions | disposable roots; zero raw-secret findings |
| AC-18 | workflow ledger and service evidence | gates reported separately; no unauthorized merge |
| AC-19 | artifact fault-injection and retention tests | atomic digest-bound bytes; exact safe cleanup |
| AC-20 | reviewer capability and identity tests | separate read-only process/thread; frozen review subject |
| AC-23 | contract construction and rejection tests | valid immutable context; empty, malformed, and unknown-version inputs denied |
| AC-24 | focused port tests plus Cargo metadata inspection | ports depend only on contracts; acyclic dependency direction |
| AC-25 | exhaustive Rust policy matrices plus dependency/I/O inspection | stable allow/deny reasons; no partial subject, cross-project, stale authority, or protected-surface bypass |
| AC-26 | shared-contract and pure Registry lifecycle/reservation/block/receipt/replay matrices plus Policy composition tests | deterministic immutable receipts; zero-mutation duplicate denial; defensive collision suspension; full-head stale/substitution/runtime denial; no I/O |
| AC-27 | shared-contract plus pure Task Ledger append/retry/replay/diagnostic/resource matrices and Policy composition tests | deterministic fake events/receipts; corruption and stale-head denial; full resource receipt/current-head substitution denial; no I/O or durability claim |
| AC-28 | shared-contract plus pure Writer Lease planning/retry/replay/recovery/admission matrices and Policy composition tests | deterministic fake transitions/receipts; overflow, corruption, stale-head, recovery, and full authority substitution denial; no I/O or durability claim |
| AC-29 | shared-contract plus pure Approval Verifier challenge/proof/nonce/time/retry/replay/checkpoint matrices and Policy composition tests | deterministic fake receipts/current heads; subject, identity, trust-lane, expiry, replay, rollback, and R3-without-review-owner denial; no live auth or durability claim |
| AC-30 | shared-contract plus pure Artifact Store object/reference/bytes/quota/authority/idempotency/replay/checkpoint/delete-claim matrices | deterministic fake receipts/current heads; project, producer, generation, aggregate bounds, digest, owner action/scope, claim token, unknown outcome, reconciliation, replay, and rollback denial; no filesystem or durability claim |
| AC-31 | shared gateway contracts plus canonical codec, fake peer/service loopback, retry/substitution, role, binding, redaction, and fault matrices | deterministic typed fake requests/replies; zero service call for malformed/protected/over-limit input; no listener, OS auth, OpenClaw load, database, provider, product, or live compatibility claim |
| AC-32 | shared Store contracts/port plus deterministic physical-head/replay fake, canonical digest, scope/authority/head/substitution/capacity/fault matrices | atomic visibly non-durable fake receipts; exact retry and unknown-outcome convergence; no SQL, driver, connection, migration execution, domain/provider/product I/O, or durability claim |
| AC-33 | exact manifest/runner/verifier unit matrices plus owned disposable PostgreSQL 17.10 apply/concurrency/permission/restart harness | checksummed schema/admission foundation and database-retained evidence; no live ControlStore, durable receipt, leader activation, user database, provider, product, or public-network claim |
| AC-34 | Store v1/v2 contract matrices, exact-prefix migration upgrade tests, and marker-owned PostgreSQL 17.10 live transaction/concurrency/retry/restart/permission harness | durable physical receipt and head evidence only; no domain repository, Guardian activation, production target, provider/product, or release claim |
| AC-35 | Task Ledger pure planner/checkpoint parity plus exact schema-v3 migration and marker-owned PostgreSQL 17.10 Ledger append/outbox/concurrency/fault/restart/corruption harness | durable atomic command/event/projection/outbox and byte-identical historical Store replay; no effect delivery, live resource observation, other repository, production, or release claim |
| AC-36 | Project Registry 1.1 observation/request/authority-receipt/command-result golden vectors; 1.2 vacant `0` and strict `1..N` planner/checkpoint/record-set vectors; exact 103-byte logical-state/Fake-Live digest fixtures; self-consistency-versus-retained-checkpoint rollback tests; schema-v4 PostgreSQL 17.10 global transaction/concurrency/fault/restart/corruption harness | acyclic command-core -> logical bytes -> result checkpoint -> record-set -> transaction/persistence commitments; complete bounded durable history and independently retained current checkpoint; serialized identity ownership and byte-identical Store/Ledger compatibility; no live Windows/Git inspection, Workspace Git, Scope Check, production, or release claim |
| AC-37 | contracts/ports/orchestrator call-order tests, exact MCP tool-list/schema tests, compatibility-wrapper parity, official Codex app-server acceptance, isolated Git fixture, fixed test, local commit and separate PostgreSQL restart/status replay | typed intent-before-effect and outcome-after-effect evidence; exactly two zero-parameter MCP tools; no caller shell/SQL/path/credential input; one verified commit and replayed terminal receipt; scripted evidence remains distinguishable from official live evidence |

## Human Decisions

- The Rust-owned writable Codex topology and the V2 amendment direction are
  already approved.
- The responsible user explicitly approved the SPEC-002 v25 / ADR-021
  delivery-composition amendment before this implementation window; v26
  records its typed-Codex-port clarification: Contracts 1.10, Ports 1.6, pure
  Orchestrator 2.1, `latticed` 1.0, its two fixed
  zero-parameter MCP tools, and the `lattice-runtime` compatibility wrapper may
  proceed without another routine review prompt.
- Routine bounded local implementation, dependency setup, disposable database
  verification, and exact-version capability preflights proceed without
  repeated chat approval when their ticket contains the safety boundary and
  verification.
- Account or credential changes, payment, public exposure, irreversible
  deletion, destructive/incompatible migrations, security-control changes,
  and protected release promotion remain on authenticated protected surfaces.
- A direct merge to the primary branch remains separately authorized.

## Open Questions

Resolved on 2026-07-29:

- The Rust core owns the single writable Codex app-server process/thread.
- OpenClaw remains the thin normal gateway.
- The user approved ADR-004 through ADR-007 and the V2 module direction by
  replying `好 開始執行`.

No material question blocks TASK-032 or later safe, bounded local work.
Protected actions listed above remain fail-closed.
