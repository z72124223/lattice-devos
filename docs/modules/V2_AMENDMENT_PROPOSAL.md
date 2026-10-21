# V2 Module Constitution Amendment Proposal

- Status: `APPROVED`
- Date: 2026-07-29
- Responsible human: user
- Specification: `SPEC-002`
- ADRs: `ADR-004` through `ADR-007`

The responsible user approved this direction on 2026-07-29 by replying
`好 開始執行` after the first Rust implementation slice was restated. Each
implemented V2 module still requires its own active `MODULE_CONSTITUTION.md`.

## Compatibility Result

The V2 request conflicts with every current V1 module set as a whole:

- the control core changes from Node.js to Rust;
- durable ledger/lease ownership moves from files to PostgreSQL;
- the Fake Runtime becomes a required test adapter rather than the product
  ceiling;
- OpenClaw changes from an inert scaffold to a thin live local gateway;
- Codex, Graphify, Hermes, Codebase Memory, and guarded self-upgrade become
  product modules;
- project-specific policy is replaced by registered-project isolation.

The invariant intent of V1 remains valuable and is preserved below.

## Existing Module Amendments

### `task-domain` 1.0 -> 2.0

- Mission retained: immutable Task Spec, hash subject, states, transitions, and
  pure validation.
- Changes:
  - define Rust-owned V2 domain contracts;
  - add generic registered-project identity and versioned capability requests;
  - add V1 read/characterization compatibility;
  - keep I/O forbidden.
- Invariants retained: deterministic canonical hash, mutable status excluded,
  unknown versions/states/transitions fail closed.
- New gates: Rust golden fixtures must reproduce every retained V1 hash and
  transition result before V2 behavior is added.

### `task-ledger` 1.0 -> 2.0

- Mission retained: single durable, append-only, verified, replayable
  control-plane truth.
- Changes:
  - replace file persistence with PostgreSQL transactional event-store ports;
  - own event/receipt semantics, canonical bytes, hash/replay rules, and
    reducer/projection contracts;
  - require `postgres-store` to implement stream-head, command-receipt,
    append/outbox atomicity, and projection persistence without owning event
    meaning;
  - retain immutable V1 file readers only for compatibility evidence.
- Invariants retained: exact sequence, predecessor hash, idempotency,
  sanitization, corruption denial, no second mutable task truth.
- New gates: transaction failure, retry, unknown commit, concurrent append,
  restart, projection mismatch, and dry-run import tests.

### `policy-engine` 1.0 -> 2.0

- Mission retained: pure, deterministic, default-deny authorization.
- Changes:
  - remove every project-specific action or name;
  - evaluate registered project, provider capabilities, external cost/network,
    memory promotion, and upgrade capability deltas;
  - distinguish proposal/test/shadow/activation permissions.
- Invariants retained: only Implementer receives product-code write,
  exact-subject approvals, unknown authority denies.
- New gates: cross-project isolation, provider capability drift, memory
  non-authority, self-approval, and protected-upgrade matrices.

### `workspace-git` 1.0 -> 2.0

- Mission retained: safe worktree identity, Git/filesystem evidence, and
  non-conflicting integration.
- Changes:
  - lease/fencing semantics move to `writer-lease`;
  - consume a verified lease/fencing capability before creating or mutating a
    worktree;
  - local file/process lock remains defense in depth;
  - project roots come from `project-registry`.
- Invariants retained: one active product writer, argument arrays, worktree
  containment, identity-proven cleanup, no automatic conflict resolution.
- New gates: database/process split-brain, overflow, suspect lease, stale epoch,
  restart, and platform containment tests.

### `scope-check` 1.0 -> 1.1

- Mission, data ownership, and read-only detection contract remain unchanged.
- Changes:
  - remove Node.js as a constitutional dependency;
  - accept language-neutral normalized Git evidence from the Rust workspace
    module;
  - use registered project identity rather than a named forbidden project.
- Gates retained: canonical paths, both sides of rename, links/junctions,
  forbidden precedence, stable manifests, and no mutation.

### `orchestrator-runtime` 1.0 -> 2.0

- Mission retained: one command entry and ordered policy/evidence/effect flow.
- Changes:
  - Rust orchestrator plus fake and live adapter ports;
  - PostgreSQL transaction/outbox boundaries;
  - exact binary/version/digest and schema-bound component observations plus
    explicit feature probes and freshness;
  - real timeout, stop, reconciliation, and daemon-epoch behavior;
  - improvement and release-candidate workflows.
- Invariants retained: intent before effect, one Implementer, lease revoked
  before review, separate merge approval, first failed gate stops progress.
- New gates: adapter unavailable/malformed/duplicate/timeout/cancel, outbox
  recovery, restart, and end-to-end self-improvement tests.

### `openclaw-adapter` 1.0 -> 2.0

- Mission retained: own the authenticated OpenClaw package and `/lattice`
  command without owning orchestration state.
- Changes:
  - replace inert response with a versioned, typed, local IPC client;
  - expose submit/plan/status/approve/reject/stop only;
  - do not own the writable Codex harness under the recommended ADR-006
    topology;
  - allow protected release approval to be initiated/displayed, but never
    accept an ordinary gateway session/token as sufficient release authority.
- Invariants retained: one command registration, no direct database/Git/product
  mutation, no static-to-live compatibility claim.
- New gates: authenticated IPC, arbitrary-command rejection, daemon
  unavailable/version mismatch, redaction, and stop/status behavior.

## New Module Constitution Proposals

### `lattice-core-bootstrap` 1.0

- Mission: expose an inert compile-time manifest of the approved platform
  components for the first buildable Rust slice.
- Non-goals: orchestration, persistence, adapter execution, policy, or
  self-upgrade.
- Owned data: no durable data; only stable component identifiers and bootstrap
  modes.
- Public contracts: platform identity and the component manifest.
- Required gates: focused Rust tests, no external dependency, architecture
  review, and no I/O side effects.
- Approval: approved with TASK-008 by the user on 2026-07-29.

### `lattice-cli` 1.0

- Mission: render the inert manifest for local inspection and recovery.
- Non-goals: normal gateway ownership, task execution, approval, persistence,
  provider invocation, installation, or deployment.
- Owned data: no durable data; only argument parsing and output rendering.
- Public contracts: accept exactly `lattice status`; reject every other
  argument sequence.
- Required gates: CLI tests, executable positive/negative smoke checks, and
  dependency review.
- Compatibility: its direct `lattice-core` dependency is limited to the inert
  v1 bootstrap manifest. Any operational command must use approved local IPC
  and a versioned constitution amendment.
- Approval: approved with TASK-008 by the user on 2026-07-29.

### `project-registry` 1.0

- Mission: own canonical project/repository identity and the
  register/suspend/move/reconcile lifecycle.
- Non-goals: Git mutation, worktree creation, policy authorization, provider
  invocation, or physical SQL ownership.
- Owned data: project identity semantics, canonical Windows root, repository
  identity, file identity, lifecycle state, and duplicate/move reconciliation.
- Public contracts: inspect candidate root, register, resolve, suspend,
  reconcile move/identity drift, and return an immutable project snapshot.
- Invariants:
  1. aliases, case folding, links/junctions, and file identity cannot register
     the same repository as independent projects;
  2. moved or identity-drifted repositories block execution until reconciled;
  3. no project-specific name/action is embedded in platform policy;
  4. every returned root is canonical and snapshot-bound.
- Allowed dependencies: contracts, filesystem-identity and repository-inspection
  ports.
- Forbidden dependencies: provider adapters, policy mutation, task execution,
  product writes.
- Required gates: Windows path/case/link/junction fixtures, duplicate roots,
  repository replacement/move, suspension, cross-project isolation, restart.

### `writer-lease` 1.0

- Mission: own project lease, fencing, daemon-instance/epoch validation,
  runtime-admission semantics, suspect-holder, revoke, and reacquire semantics.
- Non-goals: worktree/Git operations, policy decisions, process killing,
  physical tables, or direct provider invocation.
- Owned data: legal lease/epoch states, DB-clock rules, holder identity,
  fencing-token lifecycle, and recovery evidence.
- Public contracts: acquire, heartbeat, inspect, mark suspect, prove
  death/replaced epoch, revoke, release, validate daemon/lease/admission
  authority.
- Invariants:
  1. one active product writer per project;
  2. counters/epochs are monotonic non-wrapping `BIGINT` values;
  3. every daemon-authorized durable mutation validates active daemon
     instance/epoch and runtime-admission mode; product writes also validate
     current fencing token;
  4. expiry alone never proves holder death;
  5. revoked tokens/epochs are never reused.
- Allowed dependencies: contracts, DB-clock/lease persistence ports,
  process-identity evidence.
- Forbidden dependencies: Git/worktree mutation, provider adapters, policy
  ownership, arbitrary SQL.
- Required gates: concurrency, overflow, stale live connection, all repository
  mutation families, `ACTIVE/DRAINING/CANARY/STOPPED` matrices, suspect holder,
  PID reuse/start identity, epoch switch/rollback, restart/reconciliation.

### `approval-verifier` 1.0

- Mission: verify exact-subject, authenticated, expiring, one-use approvals and
  isolate the protected release-approval trust root.
- Non-goals: deciding policy, creating approval subjects, accepting silence,
  task execution, release activation, or trusting a candidate-supplied hash.
- Owned data: approval-subject schema, actor/channel/session binding, nonce
  subject/expiry semantics, signature/MAC verification, and authority class.
- Public contracts: issue challenge, canonicalize subject, verify receipt,
  produce a verified receipt/subject hash for guardian claim, explain rejection.
- Invariants:
  1. approval binds actor/authority/channel/session, target/revision/spec,
     manifest/source/binary/migration hashes, schema/policy/capability delta,
     slot/epoch, nonce, issue time, and expiry;
  2. different subject, stale expiry, unknown authority, or reused nonce denies;
  3. the candidate and normal daemon cannot access the release trust root;
  4. normal OpenClaw task IPC alone cannot satisfy protected release approval;
  5. nonce consumption belongs only to the guardian-only `claim_activation`
     store transaction, never a separate verifier call.
- Allowed dependencies: contracts, OS-authenticated identity/ACL port,
  cryptographic verifier, narrow approval persistence port.
- Forbidden dependencies: product/Git mutation, provider invocation, general
  SQL, policy or release decisions.
- Required gates: identity/session substitution, altered digest, replay,
  expiry/clock boundary, stolen normal IPC token, restart, trust-root access.

### `postgres-store` 1.0

- Mission: implement PostgreSQL tables, indexes, roles, transactions,
  migrations, event/outbox/receipt atomicity, projections, and physical
  repositories behind Rust ports.
- Non-goals: policy decisions, Git, provider invocation, arbitrary SQL from
  adapters, secret storage, inventing event/memory/lease/project transitions,
  or acting as the application orchestrator.
- Owned data: the `control`, `memory`, and rebuildable `readmodel` schemas
  approved by ADR-005.
- Public contracts: migrate/verify version; append command/event; read verified
  stream; claim/complete effect; acquire/inspect/reconcile/release lease; store
  evidence references; generic memory repositories; release/epoch procedures;
  read/rebuild projections.
- Invariants:
  1. one transaction binds every logically atomic mutation;
  2. append-only authoritative rows are never silently rewritten;
  3. service roles are least privilege;
  4. `LISTEN/NOTIFY` is never queue truth;
  5. except for guardian-only release/epoch procedures, every daemon-authorized
     mutation checks active instance, epoch, and runtime admission in the same
     transaction; direct table DML cannot bypass it;
  6. `claim_activation` alone atomically binds verified receipt/subject hashes,
     consumes the nonce, appends the claim, and sets `DRAINING`;
  7. outbox delivery is at least once and requires effect idempotency or
     reconciliation;
  8. migrations are checksum/version/compatibility checked.
- Allowed dependencies: PostgreSQL driver, contracts, ports,
  cryptographic/canonical serialization primitives.
- Forbidden dependencies: OpenClaw, Codex, Graphify, Hermes, Git, product
  repositories.
- Required gates: disposable database migrations, concurrency, restart,
  transaction fault injection, replay, least-privilege, backup/restore plan,
  and secret-leak checks.

### `artifact-store` 1.0

- Mission: own content-addressed artifact bytes, atomic writes, digest
  verification, retention, and exact safe cleanup outside product roots.
- Non-goals: deciding artifact meaning/trust, product source mutation, policy,
  general file storage, or owning PostgreSQL metadata semantics.
- Owned data: LATTICE artifact-root bytes, staging identities, digest/path
  mapping, and deletion receipts.
- Public contracts: stage stream, finalize by digest, open verified read,
  inspect, retain/release reference, sweep exact eligible objects.
- Invariants:
  1. temporary write, digest verification, durable flush, and atomic rename
     precede publication;
  2. content paths derive from algorithm/digest and never user filenames;
  3. product roots are never artifact roots or cleanup targets;
  4. cleanup requires authoritative retention/reference evidence and exact
     ownership.
- Allowed dependencies: contracts/ports, owned filesystem root, artifact
  metadata port.
- Forbidden dependencies: provider-specific semantics, Git mutation, policy,
  arbitrary recursive deletion.
- Required gates: partial write/crash, digest mismatch, concurrent duplicate,
  path traversal/link, retention race, unknown ownership, exact cleanup.

### `codex-adapter` 1.0

- Mission: own one exact-version-pinned, schema-validated Codex app-server
  process/thread contract for an approved task attempt and normalize evidence.
- Non-goals: policy, approval authority, task state, database writes, second
  thread ownership, or direct merge/promotion.
- Owned data: ephemeral process/thread binding, capability snapshot, normalized
  run receipt; durable evidence remains in PostgreSQL through Orchestrator.
- Public contracts: preflight, start/fork/resume under policy, run turn,
  respond to permission request through callback, interrupt, terminate, status.
- Invariants:
  1. one writable owner per process/thread/worktree/lease tuple;
  2. exact binary/digest, same-binary schema hash, and explicit required feature
     probes must all pass;
  3. a dedicated LATTICE `CODEX_HOME` is verified after initialize;
  4. RPC allowlist, fixed permission profile, OS/process containment, and
     post-run Scope Check enforce worktree boundaries;
  5. token usage is recorded when emitted; monetary cost is derived separately
     or remains unknown;
  6. completion/interrupt ambiguity is reconciled, never assumed.
- Allowed dependencies: contracts, process/stdio protocol, injected policy and
  evidence callbacks.
- Forbidden dependencies: general database credentials, OpenClaw/Hermes
  internals, merge authority, release activation.
- Required gates: fake protocol server, generated-schema comparison,
  overload/backpressure, malformed stream, duplicate/out-of-order events,
  dedicated-home identity, permission, worktree escape, stop, crash, and
  exact-version live preflight.

### `review-runtime` 1.0

- Mission: obtain independent, read-only review evidence against a frozen
  change/evidence/acceptance subject.
- Non-goals: product mutation, task implementation, approval authority,
  memory/release promotion, or reusing Implementer capability.
- Owned data: ephemeral reviewer identity/process/thread binding and normalized
  review receipt; durable receipt storage is supplied through ports.
- Public contracts: preflight reviewer, bind frozen subject, run review,
  interrupt, normalize findings/recommendation/evidence.
- Invariants:
  1. reviewer has no product writer lease or mutation capability;
  2. reviewer process/thread/profile differs from the Implementer's;
  3. reviewer set and acceptance/evidence hashes are fixed before review;
  4. the Implementer/runtime that produced a change is never its sole
     acceptance authority;
  5. reviewer output is evidence only. Task Packet acceptance plus
     Policy/Orchestrator (and responsible human where required) decide the gate.
- Allowed dependencies: contracts/ports, read-only snapshot/artifact inputs,
  approved provider client.
- Forbidden dependencies: product Git writes, lease acquire, policy/memory/
  release mutation, general database credentials.
- Required gates: capability separation, subject drift, missing evidence,
  same-thread reuse, malformed findings, timeout/stop, provider unavailability.

### `graphify-adapter` 1.0

- Mission: read an immutable product snapshot and produce/query versioned
  code-graph artifacts in a separate LATTICE-owned output root.
- Non-goals: product mutation, truth/approval, inference promotion, mandatory
  vector search, `graphify install`/hooks, live database introspection,
  external integration, or semantic model use in the first slice.
- Owned data: ephemeral invocation plus snapshot metadata/artifact references;
  artifact bytes belong to `artifact-store`.
- Public contracts: capability preflight, build code-only snapshot, query/path/
  explain against a named snapshot, return provenance-labeled edges.
- Invariants:
  1. source snapshot is immutable during extraction;
  2. extracted/inferred/ambiguous labels survive normalization;
  3. graph output cannot independently authorize scope or changes;
  4. all writes target LATTICE artifact staging, never the source root;
  5. rebuildability is tested for pinned input/tool/config; byte identity is
     not presumed;
  6. adapter has no product-write or DB-write credential.
- Allowed dependencies: contracts/ports, read-only process runner, injected
  `ArtifactStagingPort`. The composition root connects that port to
  `artifact-store`; the Graphify adapter never calls a concrete adapter.
- Forbidden dependencies: policy mutation, Codex/Hermes, writer lease, memory
  promotion.
- Required gates: known code fixture, changed-tree/source-write rejection,
  output-root preflight, install/hook denial, artifact hashes, repeated-build
  comparison, label preservation, code-only/no-network/live-DB proof,
  timeout/cancel/malformed output.

### `hermes-adapter` 1.0

- Mission: obtain bounded research, reflection, and improvement candidates from
  a contained Hermes process with read-only product input.
- Non-goals: product coding, second planning authority, authoritative memory,
  skill/policy promotion, account setup, or writable Codex runtime.
- Owned data: ephemeral run/session binding and accepted candidate envelope;
  Hermes-internal state and arbitrary raw output are not LATTICE-owned truth.
- Public contracts: capability preflight, submit bounded run, stream status,
  stop, validate/quarantine output, return schema-valid candidate envelope with
  provenance.
- Invariants:
  1. dedicated `HERMES_HOME` is state separation, not security isolation;
  2. independently enforced whole-process OS containment provides read-only
     product input, separate candidate output, and no Git/DB credentials;
  3. arbitrary/malformed/provenance-free output is rejected or quarantined;
  4. every accepted envelope remains untrusted candidate data;
  5. Hermes memory/skill/guard settings are defense in depth only.
- Allowed dependencies: contracts/ports, dedicated sandbox/process/API client,
  artifact candidate output.
- Forbidden dependencies: Codex writable thread, Git mutation, memory
  acceptance, approval, release promotion.
- Required gates: adversarial prompts, attempted mutation, memory/skill write,
  arbitrary/schema-drift/provenance-free output, timeout/stop, profile state
  separation, credential denial, and OS-boundary evidence.

### `codebase-memory` 1.0

- Mission: manage project-isolated, provenance-bound memory candidates,
  revisions, review, retrieval, supersession, and expiry.
- Non-goals: authorization, scope, leases, release state, raw transcript truth,
  or automatic trust based on source name.
- Owned data: memory record semantics, provenance links, review transitions,
  retrieval policy/evidence; physical persistence is supplied by
  `postgres-store`.
- Public contracts: propose, quarantine, accept, reject, supersede, expire,
  search, explain retrieval, report contradictions.
- Invariants:
  1. accepted records have immutable evidence sources;
  2. fact/observation/inference/decision/failure/preference remain distinct;
  3. cross-project retrieval denies by default;
  4. unapproved/stale/superseded/poisoned content is not trusted context;
  5. memory cannot grant capability or override higher authority;
  6. `PREFERENCE` acceptance requires authenticated user evidence; model
     suggestions remain `INFERENCE/CANDIDATE`.
- Allowed dependencies: contracts, store/retrieval ports, approved snapshot
  metadata.
- Forbidden dependencies: direct provider/Git/OpenClaw calls, policy or lease
  mutation.
- Required gates: provenance, contradiction, poisoning, prompt injection,
  cross-project isolation, expiry, supersession, no-answer false positives, and
  retrieval audit plus Traditional Chinese, mixed-language, Rust symbol/path,
  error-code, and exact-filename benchmark.

### `self-upgrade-guardian` 1.0

- Mission: verify immutable release manifests, run read-only shadow health,
  coordinate a recoverable A/B activation saga, monitor, stop, and roll back.
- Non-goals: product coding, task intake, policy/constitution approval,
  destructive database migration, credentials, or autonomous replacement of
  the guardian itself.
- Owned data: local immutable slot identities, checksum-bound boot projection,
  activation coordinator, guardian health contract, and rollback receipts.
  PostgreSQL release/daemon-epoch events remain durable truth.
- Public contracts: inspect slots, verify manifest, shadow start/stop, drain,
  atomically `claim_activation`, append release-stream intents/outcomes, change
  admission/epoch through narrow procedures, activate, health, finalize,
  rollback, recovery status.
- Invariants:
  1. active bundle is never overwritten in place;
  2. only the guardian changes the active slot;
  3. candidates cannot append control events or acquire product leases during
     shadow;
  4. every external saga step has a durable intent before and outcome after;
  5. daemon epochs prevent old writers after activation and never decrement;
  6. exact release approval binds actor/session, full manifest, deltas, slot,
     epoch, nonce, issue time, and expiry;
  7. claim atomically consumes nonce, appends `ACTIVATION_CLAIMED`, and sets
     `DRAINING`;
  8. `OLD_DAEMON_DRAINED` proves zero leases, zero
     claimed/running/unknown effects, reconciled outcomes, and no writable
     Codex child/process tree;
  9. `EPOCH_ACTIVATED` sets `CANARY`; only the reserved system health stream is
     writable until finalization sets `ACTIVE`;
  10. rollback only targets a schema-compatible verified slot and starts it at a
     higher epoch;
  11. `RECONCILIATION_REQUIRED` admits no daemon mutation/effect;
  12. first-version A/B activation performs no schema migration.
- Allowed dependencies: contracts, read-only status plus narrowly scoped
  release/daemon-epoch procedure port, `approval-verifier`, process/service
  control, cryptographic manifest verification.
- Forbidden dependencies: product source/worktree edits, general task
  orchestration, task/memory streams, general SQL credentials, policy change,
  destructive migration.
- Required gates: corrupt manifest, altered/replayed/expired approval, crash
  loop, atomic nonce/claim/admission fault injection, every partial activation
  state, task/lease/effect admission during drain, complete drain proof, every
  durable repository mutation in canary/stale-epoch modes, stale live
  connection, boot/DB disagreement, system-stream write canary, health
  disagreement, power/process interruption simulation, incompatible schema,
  and rollback drill.

## Dependency And Ownership Rules

In this diagram, `A -> B` means **A depends on B**:

```text
OpenClaw -> local IPC -> latticed

lattice-ports -> lattice-contracts
lattice-policy -> lattice-contracts

lattice-orchestrator
  -> lattice-contracts
  -> lattice-policy
  -> lattice-ports

lattice-cli (bootstrap v1 only)
  -> lattice-core-bootstrap inert manifest

postgres-store/workspace-git/project-registry/writer-lease
artifact-store/codebase-memory/approval-verifier
codex-adapter/review-runtime/graphify-adapter/hermes-adapter
  -> lattice-ports
  -> lattice-contracts

latticed -> lattice-orchestrator + concrete adapters

self-upgrade-guardian
  -> narrow release/daemon-epoch/status ports
  -> approval-verifier
```

- Cross-module mutable data has one owner.
- Adapters do not call each other.
- The Orchestrator owns workflow order; Task Ledger owns event meaning; Project
  Registry owns repository identity; Writer Lease owns fencing/epoch semantics;
  Codebase Memory owns memory transitions; Artifact Store owns bytes; the
  PostgreSQL adapter owns physical persistence mechanics; Policy owns
  allow/deny; Approval Verifier owns approval proof; the guardian owns
  activation.
- Concrete adapters implement ports and do not depend on Orchestrator.
- `latticed` is the normal composition root. The guardian is a separate minimal
  composition root limited to recovery/release authority.
- New shared interfaces must stabilize before tickets are marked
  `parallel_safe`.

## Approval Requested

Approval means:

1. ADR-004 through ADR-007 may move from `proposed` to `accepted`;
2. existing constitutions may receive the listed versioned amendments;
3. new constitutions may be created at
   `docs/modules/<module-id>/MODULE_CONSTITUTION.md`;
4. SPEC-002 may be updated from blocked draft to approved/ready;
5. dependency-aware V2 tickets may be created.

Approval of the module direction alone does **not** authorize code
implementation, installation, account or credential action, database
creation/migration, model use, payment, push, merge, publication, deployment,
or public network exposure. The separate approval record below authorizes only
the named local bootstrap slice.

## Approval Record

- Decision: approved
- Date: 2026-07-29
- Approver: responsible user
- Evidence: direct reply `好 開始執行`
- Authorized implementation scope: local, reversible Rust/Cargo workspace
  bootstrap, active module constitutions, one bootstrap ticket, fake adapter
  shells, and an unexecuted PostgreSQL schema draft.
- Still excluded: software installation, database connection or mutation,
  credentials, model calls, payment, publication, push, merge, deployment, and
  public network exposure.
