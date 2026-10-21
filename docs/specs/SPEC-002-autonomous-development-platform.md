---
spec_id: SPEC-002
status: ready
version: 2
supersedes_for_new_work: SPEC-001
modules:
  - module_id: task-domain
    constitution_version: 2.0
  - module_id: task-ledger
    constitution_version: 2.0
  - module_id: policy-engine
    constitution_version: 2.0
  - module_id: project-registry
    constitution_version: 1.0
  - module_id: writer-lease
    constitution_version: 1.0
  - module_id: workspace-git
    constitution_version: 2.0
  - module_id: scope-check
    constitution_version: 1.1
  - module_id: orchestrator-runtime
    constitution_version: 2.0
  - module_id: openclaw-adapter
    constitution_version: 2.0
  - module_id: approval-verifier
    constitution_version: 1.0
  - module_id: postgres-store
    constitution_version: 1.0
  - module_id: artifact-store
    constitution_version: 1.0
  - module_id: codex-adapter
    constitution_version: 1.0
  - module_id: review-runtime
    constitution_version: 1.0
  - module_id: graphify-adapter
    constitution_version: 1.0
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
- Provider-specific behavior remains behind versioned ports. External adapters
  never receive general PostgreSQL credentials.
- Project Registry, Writer Lease, Task Ledger, Codebase Memory, Artifact Store,
  Review Runtime, and Approval Verifier have separate domain ownership; the
  PostgreSQL adapter supplies physical persistence without deciding their
  legal transitions.
- Local IPC uses an authenticated OS-local transport and does not listen on a
  public interface by default.

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
| task-domain | 2.0 | Rust contract and V2 schema; V1 read compatibility |
| task-ledger | 2.0 | Event semantics/replay contract; physical storage moves to PostgreSQL |
| policy-engine | 2.0 | Generic project/capability/upgrade policy; removes specific-project action |
| project-registry | 1.0 | New canonical repository identity and registration lifecycle |
| writer-lease | 1.0 | New lease/fencing/daemon-epoch domain owner |
| workspace-git | 2.0 | Worktree/Git/filesystem evidence only; consumes lease authority |
| scope-check | 1.1 | Language-neutral contract; mission remains detection-only |
| orchestrator-runtime | 2.0 | Real adapter supervision and transactional effect outbox |
| openclaw-adapter | 2.0 | Inert scaffold becomes a thin local IPC gateway |
| approval-verifier | 1.0 | New exact-subject identity/nonce/trust-root boundary |
| postgres-store | 1.0 | New physical transaction, table, index, migration, and projection adapter |
| artifact-store | 1.0 | New content-addressed artifact-byte owner |
| codex-adapter | 1.0 | New owner of one writable app-server process/thread contract |
| review-runtime | 1.0 | New independent read-only review boundary |
| graphify-adapter | 1.0 | New read-only-source graph/artifact boundary |
| hermes-adapter | 1.0 | New contained research/candidate boundary |
| codebase-memory | 1.0 | New provenance/review/retrieval domain |
| self-upgrade-guardian | 1.0 | New A/B activation, health, and rollback boundary |
| lattice-core-bootstrap | 1.0 | Inert compile-time component manifest for the first Rust slice |
| lattice-cli | 1.0 | Read-only bootstrap inspection/recovery command; no runtime authority |

The user approved this module direction and the local bootstrap slice on
2026-07-29. Only modules with an active constitution and a current ticket may
be implemented; all live capabilities retain their separate approval gates.

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
- [ ] AC-03: PostgreSQL append atomically enforces command idempotency,
  expected sequence, predecessor/event hashes, event append, stream head, and
  outbox intent, and terminal receipt under a unique stream/command key.
- [ ] AC-04: Replay produces the same projection after restart; corruption or
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
- [ ] AC-07: OpenClaw can submit/status/approve/stop through typed local IPC
  without direct database, Git, provider, or product-path access.
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
- [ ] AC-22: Memory retrieval meets an approved benchmark for Traditional
  Chinese, mixed Chinese/English, Rust symbols/paths, error codes, filenames,
  and no-answer behavior before an indexing strategy is accepted.

## Verification Plan

| Criterion | Verification method | Expected evidence |
|---|---|---|
| AC-01 | repository text/rule scan | zero active project-specific policy references |
| AC-02, AC-21 | manually approved Rust golden/characterization tests | same retained V1 results plus frozen V2 canonical bytes |
| AC-03 to AC-05 | disposable PostgreSQL integration/concurrency tests | atomic append, replay, lease, overflow, restart evidence |
| AC-06 | disposable registry/Git repositories and platform path fixtures | identity lifecycle and exact deny/pass manifests; no external path touched |
| AC-07 | fake local IPC/plugin contract tests | typed commands only; no forbidden dependency |
| AC-08 | fake server then exact-version live preflight | identity/digest, same-binary schema hash, feature probes, dedicated home, event/stop/scope evidence |
| AC-09 | immutable read-only source fixture and separate artifact root | no source writes/install/hooks; provenance-labeled derived output |
| AC-10 | OS sandbox/capability/adversarial tests | only valid envelopes accepted; mutation/credential attempts denied |
| AC-11 to AC-12, AC-22 | memory poisoning, isolation, provenance, multilingual/symbol retrieval tests | conservative accepted-only results, measured threshold, and audit |
| AC-13 to AC-16 | deterministic end-to-end, approval replay, power-loss, epoch, and A/B rollback drills | no self-approval, safe stop, recoverable saga, write canary, rollback |
| AC-17 | test-root and secret-leak assertions | disposable roots; zero raw-secret findings |
| AC-18 | workflow ledger and service evidence | gates reported separately; no unauthorized merge |
| AC-19 | artifact fault-injection and retention tests | atomic digest-bound bytes; exact safe cleanup |
| AC-20 | reviewer capability and identity tests | separate read-only process/thread; frozen review subject |

## Human Decisions

- Approve or reject the Rust-owned writable Codex app-server topology.
- Approve versioned module amendments before constitution or implementation
  changes.
- Later authorize exact external component installation and capability
  preflights.
- Later approve database identities/credentials, incompatible migrations,
  protected capability changes, initial release promotions, and merges.

## Open Questions

Resolved on 2026-07-29:

- The Rust core owns the single writable Codex app-server process/thread.
- OpenClaw remains the thin normal gateway.
- The user approved ADR-004 through ADR-007 and the V2 module direction by
  replying `好 開始執行`.

No material question blocks the local bootstrap ticket. Live component
installation, database access, credentials, model calls, promotion, merge, and
deployment remain separate future approvals.
