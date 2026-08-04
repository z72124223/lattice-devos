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

## TASK-010 Canonical Mechanism Clarification

TASK-010 introduces `lattice-cjson` 1.0 as a pure technical module. It owns
only the `lattice-cjson-1` typed value-to-byte algorithm and generic
schema-domain-separated digest frame.

- Task Domain owns the Task Spec field set and current
  `lattice.task-spec/2.1` hash
  subject.
- Task Ledger continues to own event/receipt field selection, predecessor and
  event hashes, replay, and fixture semantics.
- Approval, memory, and release modules similarly own their own hash subjects.
- `lattice-contracts` remains serialization- and hashing-free.

This clarification resolves technical reuse without moving semantic ownership
or creating a Task Ledger -> Task Domain dependency.

## Existing Module Amendments

### `task-domain` 1.0 -> 2.0 (historical V2 activation)

- Mission retained: immutable Task Spec, hash subject, states, transitions, and
  pure validation.
- Changes:
  - define Rust-owned V2 domain contracts;
  - add generic registered-project identity and versioned capability requests;
  - add V1 read/characterization compatibility;
  - use exact `time` 0.3.54 parsing/formatting only for canonical UTC RFC 3339
    strings without reading the system clock;
  - keep I/O forbidden.
- Invariants retained: deterministic canonical hash, mutable status excluded,
  unknown versions/states/transitions fail closed.
- New gates: Rust golden fixtures must reproduce every retained V1 hash and
  transition result before V2 behavior is added.

### `task-domain` 2.0 -> 2.1 (TASK-011 review hardening)

- Bind one canonical three-letter uppercase accounting currency to every
  immutable Task Spec budget.
- Include that currency in canonical Task Spec bytes and `spec_hash`.
- Publish and enforce one 256-byte maximum for canonical decimal budget
  strings, with 127 integer digits and 128 fractional digits, so Task Domain
  construction and Policy mixed-scale arithmetic cannot drift.
- Fail closed on legacy 2.0 construction in active V2 work; retained V1
  characterization remains governed by its separate compatibility evidence.
- This amendment closes cross-currency budget ambiguity discovered during the
  independent TASK-011 security review.

### `task-ledger` 1.0 -> 2.0

- Mission retained: single durable, append-only, verified, replayable
  control-plane truth.
- Changes:
  - replace file persistence with PostgreSQL transactional event-store ports;
  - own event/receipt semantics, canonical bytes, hash/replay rules, and
    Ledger-owned resource reducer/projection contracts;
  - require `postgres-store` to implement stream-head, command-receipt,
    append/outbox atomicity, and projection persistence without owning event
    meaning;
  - retain immutable V1 file readers only for compatibility evidence.
- Invariants retained: exact sequence, predecessor hash, idempotency,
  sanitization, corruption denial, no second mutable task truth.
- New gates: transaction failure, retry, unknown commit, concurrent append,
  restart, projection mismatch, and dry-run import tests.

TASK-013 activates the pure semantic subset before PostgreSQL:

- complete project/snapshot/task/revision/spec/currency stream identity;
- separate request/event/head/receipt/resource hash domains;
- exact full-head append and `(stream_id, command_id)` terminal receipts;
- closed typed events plus bounded non-authoritative sanitized diagnostics;
- verified chain/resource replay and fixed-producer resource observations;
- a deterministic `RuntimeKind::Fake` in-memory owner that is explicitly
  non-durable.

Task Domain continues to own legal task-state transitions. Task Ledger has no
Task Domain dependency; future Orchestrator composition consumes both public
contracts. PostgreSQL transaction/concurrency/restart/unknown-commit/outbox
gates remain open.

### `policy-engine` 1.0 -> 2.0 (historical V2 activation)

- Mission retained: pure, deterministic, default-deny authorization.
- Changes:
  - remove every project-specific action or name;
  - consume the constructed immutable Task Spec through the explicit
    `lattice-policy -> lattice-task-domain` edge in ADR-009 rather than
    duplicating domain enums or accepting partial string maps;
  - evaluate registered project, provider capabilities, external cost/network,
    memory promotion, and upgrade capability deltas;
  - distinguish proposal/test/shadow/activation permissions.
- Invariants retained: only Implementer receives product-code write,
  exact-subject approvals, unknown authority denies.
- New gates: cross-project isolation, provider capability drift, memory
  non-authority, self-approval, and protected-upgrade matrices.

### `policy-engine` 2.0 -> 2.1 (TASK-011 review hardening)

- Bind protected release approval, evidence, and rollback to one exact Guardian
  runtime identity, trust root, daemon instance, release tuple, slot, and epoch.
- Replace naked merge/resource booleans with fresh, owner-produced,
  exact-subject facts whose project, Task Spec, effect, head, and currency
  bindings are checked.
- Add dedicated normal-runtime and Guardian recovery subjects; generic runtime
  reconciliation remains denied.
- Canonicalize protected Git branch references, bind Release Writer authority
  to the actor, and give external-cost denial fixed precedence.
- This amendment records the review-driven contract corrections; it does not
  retroactively rewrite the historical 1.0 -> 2.0 proposal.

### `policy-engine` 2.1 -> 2.2 (TASK-012 Registry receipt binding)

- Replace caller-owned registered/active/drifted fields with one shared,
  task-agnostic Project Registry authority receipt plus its exact current
  Registry head.
- Keep Task-Spec-specific `SubjectBinding` in Policy/Orchestrator and require
  exact project/snapshot/revision/digest plus fake/live runtime agreement.
- Move neutral Project ID/class/lifecycle and physical Git-ref identity
  representation to `lattice-contracts` 1.1.
- Preserve the approved dependency edge; Policy still does not depend on
  Project Registry.

### `policy-engine` 2.2 -> 2.3 (TASK-012 review hardening)

- Compare every security-relevant receipt field against the full head returned
  by an independent current Registry-owner lookup.
- Treat `receipt.head()` only as a structural projection; it cannot prove that
  a historical receipt is still current.
- Preserve the pure dependency edge through `lattice-contracts` 1.2. Future
  Orchestrator/PostgreSQL composition must authenticate and serialize the
  current lookup before live authority is claimed.
- Narrow Git revision-alias rejection to an explicit pseudo-ref denylist so
  valid uppercase branch names remain accepted.

### `policy-engine` 2.3 -> 2.4 (TASK-013 Ledger receipt binding)

- Replace caller-owned resource owner/producer strings and `fresh` Boolean with
  one shared fixed-producer Task Ledger resource receipt plus a full head
  obtained from an independent current Ledger-owner lookup.
- Compare producer/version, runtime, complete task/project/spec/stream head,
  resource revision/projection, effect claim, currency, counters, and all
  observation/receipt digests.
- Preserve the pure dependency edge through `lattice-contracts` 1.3. Policy
  has no normal/production Task Ledger dependency and cannot treat a
  self-projected head as currentness evidence. A one-way test-only Cargo
  `dev-dependency` lets the Policy integration matrix consume the fake
  Ledger owner's actual current-head lookup without moving Ledger semantics
  into Policy.
- Keep real effect admission deferred until PostgreSQL re-checks and claims the
  resource counters, daemon epoch, runtime admission, and outbox intent in one
  transaction.

### `policy-engine` 2.4 -> 2.5 (TASK-014 Writer Lease receipt binding)

- Replace caller-owned writer `active`, `current`, holder role,
  current-daemon-epoch, current-fence, and active-Implementer count fields with
  one shared fixed-producer Writer Lease authority receipt plus an optional
  full head obtained from an independent current Writer Lease owner lookup.
- Compare runtime, complete project/snapshot/task/revision/spec/attempt,
  lease/holder/worktree/process-start, daemon instance/epoch/fence, state,
  admission, revision, timestamps, and all observation/transition/receipt
  digests.
- Preserve the pure dependency edge through `lattice-contracts` 1.4. Policy
  has no normal/production Writer Lease dependency and cannot treat
  `receipt.head()` as currentness. A test-only dependency may prove
  composition using the fake owner's actual current head.
- Keep live writer/effect admission deferred until PostgreSQL authenticates and
  serializes daemon leadership, runtime admission, current lease/fence, and
  the durable mutation/effect claim in the applicable transaction.

### `policy-engine` 2.5 -> 2.6 (TASK-015 approval authority binding)

- Replace caller-owned approval subject/identity/freshness/nonce/self-approval
  verdict Booleans with one fixed-producer Approval Verifier receipt plus an
  optional complete head obtained from an independent current owner lookup.
- Compare the complete typed approval subject in the receipt with the expected
  decision subject; an opaque caller digest or sidecar subject is insufficient.
- Remove caller-owned security/architecture `ReviewChecks`. Until Review
  Runtime has its own fixed-owner receipt/current-head contract, R3 and every
  independent-review-required allow path fail closed.
- Preserve the production dependency edge through `lattice-contracts` 1.5.
  Approval Verifier is a Policy test-only dependency for actual fake-owner
  composition.
- Keep live approval claim deferred until PostgreSQL rechecks current nonce,
  database time, subject, daemon/admission, and applicable effect state in the
  same transaction.

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

### TASK-017 Gateway Contract Amendment

ADR-015 and TASK-017 activate the already approved local gateway direction as
a pure/fake slice:

- `lattice-contracts` 1.6 -> 1.7 adds neutral bounded peer/request/reply,
  action-specific target, disposition, and stable denial representations;
- `lattice-ports` 1.0 -> 1.2 changes `GatewayService` from the nominal
  `GatewayCommand -> GatewayEvidence` skeleton to server-derived peer context
  plus a complete typed request, bound typed reply, and component-free core
  error while retaining the contracts-only dependency;
- `gateway-ipc` 1.1 owns wire codec/parser limits/digests/NFC/exact-retry while
  Contracts owns neutral representation/constructor bounds; it supplies only
  a visibly fake in-memory loopback;
- `openclaw-adapter` 2.0 becomes the thin future native IPC client, but live
  plugin loading, transport, OS authentication, and compatibility remain
  MVP-2;
- `orchestrator-runtime` 2.0 owns behavior routing and cannot be duplicated by
  the codec or adapter.

The Task Spec canonical document is transported with its claimed 2.1 digest;
IPC checks canonical bytes and the mechanical digest/binding only. Task Domain
retains sole field-semantic validation. Normal gateway approval cannot encode
protected release, and task stop cannot claim terminal stop evidence.

## New Module Constitution Proposals

### `gateway-ipc` 1.1

- Mission: own the bounded canonical local gateway protocol, typed codec,
  request/reply digest, and pure in-memory fake loopback.
- Non-goals: OpenClaw/plugin execution, live transport/authentication, task
  truth, orchestration, PostgreSQL, Git, providers, credentials, or Guardian
  authority.
- Public contracts: six action-specific requests, typed bound replies,
  server-derived fake peer context, 1 MiB preflight, canonical JSON, exact
  retry/substitution, role allowlists, and redacted errors.
- Dependencies: contracts, ports, cjson, exact bounded JSON parser, and exact
  allocation-free NFC preflight predicate only.
- Approval: covered by the user's later directive to execute bounded local
  tickets through MVP-3; protected actions remain separately gated.

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

### `lattice-cjson` 1.0

- Mission: own the pure `lattice-cjson-1` value-to-byte mechanism and
  `lattice-hash-1` generic domain frame.
- Non-goals: choose Task Spec/event/approval/memory/release fields, parse wire
  JSON, own persistence, or reproduce V1 JavaScript behavior.
- Owned data: algorithm/frame IDs, typed canonical values, Unicode NFC,
  UTF-8-key ordering, minimal escaping, duplicate-normalized-key rejection,
  and SHA-256 digest bytes.
- Public contracts: canonical bytes, exact framed input, and digest; raw
  numbers are absent from the type system.
- Required gates: frozen cross-language bytes/hash, Unicode/collision/order/
  null/error fixtures, exact dependencies, and architecture review.
- Approval: approved for TASK-010 by the user's directive to execute the
  approved plan through MVP-3 on 2026-07-29.

### `lattice-contracts` 1.0

- Mission: own the smallest versioned, I/O-free Rust boundary types shared by
  ports and concrete adapters.
- Non-goals: domain transitions, orchestration, policy, serialization,
  persistence, process control, provider protocol details, or canonical JSON.
- Owned data: immutable in-process identifiers, SHA-256 references, invocation
  context, component identity, boundary classification, runtime classification,
  and normalized evidence references; no durable data.
- Public contracts: reject empty identifiers, malformed SHA-256 references,
  and unsupported versions; construct immutable invocation and evidence values.
- Required gates: focused contract tests, no external dependencies, dependency
  inspection, and architecture review.
- Approval: approved for TASK-009 by the user's `繼續其他部分` instruction on
  2026-07-29.

### `lattice-contracts` 1.0 -> 1.1 (TASK-012 shared Registry values)

- Add validated canonical `ProjectId`, closed `ProjectClass` and
  `ProjectLifecycle`, and a fully qualified local `refs/heads/*`
  `GitRefIdentity` carrying an owner-supplied physical storage digest.
- Add minimal immutable task-agnostic `ProjectAuthorityReceipt` and
  `ProjectAuthorityHead` values.
- Contracts owns representation/validation only. Project Registry retains
  lifecycle, issuance, canonical-root, repository/file identity, and mutable
  state ownership.

### `lattice-contracts` 1.1 -> 1.2 (TASK-012 review hardening)

- Fix the only accepted Registry authority producer ID and semantic producer
  version; callers cannot substitute either field.
- Expand `ProjectAuthorityHead` to mirror all security-relevant receipt
  fields: producer/version, runtime, project/snapshot, revision,
  lifecycle/class, primary ref, observation digest, and receipt digest.
- Define `receipt.head()` as a structural projection rather than currentness
  proof.

### `lattice-contracts` 1.2 -> 1.3 (TASK-013 shared Ledger values)

- Add neutral immutable full Task Ledger stream-head and checked resource usage
  representation.
- Add fixed producer `lattice-task-ledger`, semantic version `2.0`, exact
  runtime, and full resource observation receipt/head values.
- Mirror complete project/snapshot/task/revision/spec/stream/resource/claim/
  currency/counter/digest fields so Policy can perform exact equality against
  an independently obtained owner head.
- Contracts owns representation and validation only. Task Ledger retains hash,
  issuance, replay, projection, and mutable counter ownership; PostgreSQL later
  owns durable persistence.
- Use an explicit pseudo-ref denylist and retain valid uppercase local branch
  names.

### `lattice-contracts` 1.3 -> 1.4 (TASK-014 shared Writer Lease values)

- Add neutral immutable complete Writer Lease identity plus positive
  signed-BIGINT-compatible daemon epoch, fencing token, and lease-revision
  values.
- Add fixed producer `lattice-writer-lease`, semantic version `1.0`, exact
  fake/live runtime, closed runtime-admission representation, and full
  authority receipt/head values.
- Mirror every security-relevant identity/state/admission/revision/timestamp/
  observation/transition/receipt field so Policy can compare against an
  independently obtained current owner head.
- Contracts owns representation and validation only. Writer Lease retains
  transition, hashing, issuance, idempotency, recovery, and fencing ownership;
  Guardian/PostgreSQL retain admission/leadership transitions and durable
  persistence.

### `lattice-contracts` 1.4 -> 1.5 (TASK-015 shared approval values)

- Move the complete neutral typed approval subject graph shared by Policy and
  Approval Verifier into Contracts: binding, execution/cost, merge,
  preference, protected-change, guarded-release/Guardian, and upgrade-delta
  representations.
- Add fixed producer `lattice-approval-verifier`, semantic version `1.0`,
  runtime, challenge/authority identity, positive revision, availability,
  receipt, and complete authority-head values.
- Derive approval kind from the typed subject; constructors reject
  contradictory subject/trust-lane combinations and protected release outside
  the Guardian trust lane.
- Preserve Contracts as hashing-, proof-, time-, issuance-, currentness-,
  claim-, and review-authority-free representation only.

### `lattice-contracts` 1.5 -> 1.6 (TASK-016 shared artifact values)

- Add neutral project-scoped artifact object/digest/generation, bounded byte
  length and quota values, immutable reference/provenance, purpose/retention,
  availability/delete-claim status, positive revision, fixed
  `lattice-artifact-store`/`1.0` producer, receipt, complete current-head, and
  typed reference-owner/sweep-authority receipt/head representations.
- Mirror complete project/snapshot/task/revision/spec/attempt/request,
  object/generation/media/schema/bundle, source producer/runtime/binary,
  adapter/version/binary, invocation/correlation/run/sequence/produced-at/payload,
  capability/input/config/evidence, Registry/effect/daemon/admission/
  capability-owner/limit-snapshot authority, reference/purpose/retention,
  delete claim, manifest, observation, transition, and receipt fields.
- Preserve Contracts as content/manifest hashing-, producer-authentication-,
  lifecycle-, quota-decision-, issuance-, currentness-, trust-, persistence-,
  and deletion-authority-free representation only.

### `lattice-ports` 1.0

- Mission: define the typed inbound gateway service plus abstract Rust traits
  for the sole code writer, read-only knowledge lane, untrusted research lane,
  and control-store boundary.
- Non-goals: select a provider, perform I/O, own orchestration order, decide
  policy, or implement PostgreSQL durability.
- Owned data: typed port errors and trait signatures only; no runtime or
  durable data.
- Public contracts: `GatewayService`, `CodexPort`, `GraphifyPort`,
  `HermesPort`, and `ControlStore`; every port returns a lane-specific evidence
  type with a fixed component/boundary pair.
- Required gates: focused trait-contract tests and proof that the crate depends
  only on `lattice-contracts`.
- Approval: approved for TASK-009 by the user's `繼續其他部分` instruction on
  2026-07-29.

### `project-registry` 1.0

- Mission: own canonical project/repository identity and the
  register/suspend/move/reconcile lifecycle.
- Non-goals: Git mutation, worktree creation, policy authorization, provider
  invocation, or physical SQL ownership.
- Owned data: project identity semantics, canonical Windows root, repository
  identity, file identity, lifecycle state, and duplicate/move reconciliation.
- Public contracts: inspect candidate root, register, resolve, suspend,
  reconcile move/identity drift, and return an immutable project snapshot.
- TASK-012 narrows the first executable boundary to immutable fake
  `RepositoryObservation` input; real inspection ports remain later. It adds
  exact authority receipts/current heads and idempotent command receipts with
  command/request/before/after/result binding plus an expected full head for
  observe/suspend/reconcile; register has no prior head, and exact read-only
  observation still returns a receipt.
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
  TASK-012 proves the pure comparison/lifecycle/receipt subset; real Windows
  inspection, PostgreSQL restart, and loose/packed Git-ref stability remain
  future required gates.

### `project-registry` 1.0 -> 1.1 (TASK-012 review hardening)

- Require already-NFC command IDs, canonical-root text, and primary-ref text
  before canonical hashing; hidden normalization cannot change the subject.
- Reserve accepted pending identities for their owning project and reject
  front-running registration or reconciliation.
- Keep ordinary duplicate registration/reconciliation as `Denied` with no
  mutation.
- Introduce a distinct hashed `Blocked` terminal outcome for an authoritative
  observation collision. The observed `ACTIVE` project advances to a new
  `SUSPENDED` head, its colliding pending observation is cleared, and the other
  project's accepted or pending reservation is unchanged.
- Expose a fake current-head lookup for composition tests without claiming
  persistence, authentication, or live inspection.

### `project-registry` 1.1 -> 1.2 (TASK-022 durable global repository boundary)

- Preserve Project Registry 1.1's observation, request, authority-receipt, and
  command-result hash subjects, lifecycle outcomes, accepted/pending
  reservation rules, exact retry, `Denied`, and `Blocked` meaning.
  `result_digest` remains the terminal semantic command-result commitment;
  Registry 1.1 had no separate terminal-receipt or record-set hash subject.
- Add one runtime-aware verified global state with vacant, pure `plan -> apply`,
  complete first-seen command ordering, independently comparable checkpoint,
  untrusted export, and ordered replay. The deterministic Fake and PostgreSQL
  adapter must consume this same legality path; the domain owner remains
  zero-I/O.
- `RegistryCheckpoint::from_retained` reconstructs an independently read
  checkpoint without asserting currentness. Plain
  `verify_untrusted_registry_snapshot` proves only internal self-consistency;
  only `verify_untrusted_registry_snapshot_against_checkpoint` also compares
  the retained singleton and may precede durable current authority.
- Every first-seen terminal command, including zero-project-change `Denied`,
  state-changing `Blocked`, and exact read-only observation, advances the
  non-wrapping global checkpoint exactly once. Vacant high-water is `0` and
  first-seen records are exactly `1..N`; exact command/request replay advances
  neither and changed command reuse discloses no retained receipt.
- The verified state retains every complete command request/receipt, including
  collision and denial observations, and cross-checks all project projections,
  accepted/pending reservations, counts, retained bytes, runtime, and checkpoint
  chain by replay from a vacant state.
- Freeze the acyclic commitment order: checkpoint command core (ordinal,
  complete typed request, complete semantic `RegistryCommandReceipt`) -> exact
  logical-retained-state bytes -> result checkpoint -> record set -> adapter
  transaction digest -> persistence receipt. Checkpoint references, record-set
  fields, counts/retained bytes, and adapter evidence are excluded from the
  command core; checkpoint references are chain evidence, not checkpoint input.
- The logical-state object has exactly `schema_version`, `runtime`,
  `observations`, `projects`, `commands`, and `reservations`. Unique complete
  observations are digest-sorted and counted once; projects are Project-ID
  sorted and refer by digest; command cores are ordinal-sorted; reservations
  are sorted by dimension/digest/status/Project ID; optional values are
  canonical `null`; text is NFC UTF-8; numeric values are canonical decimal
  strings. Its canonical byte length excludes hash frames, counts/the byte
  field, checkpoint/record-set fields, SQL overhead, and adapter evidence.
- The exact vacant Live logical state is
  `{"commands":[],"observations":[],"projects":[],"reservations":[],"runtime":"LIVE","schema_version":"1"}`
  at 103 bytes. Frozen vacant checkpoint digests are
  `22ad9599c05ab384e720b8f1d91bdfbe1262360850602aa0f6b5fd79c1797f4f`
  for Fake and `5bb1f9d9adf7228bef7ea45cc029e79d71761c4dfed3abe086634917db38c173`
  for Live. Registry 1.1 golden vectors cover only observation/request/
  authority-receipt/command-result; Registry 1.2 adds new checkpoint and
  record-set vectors.
- Versioned bounds fail closed at 4,096 projects, 65,536 first-seen terminal
  commands, 67,108,864 logical-state canonical bytes, and 131,072 UTF-8 bytes
  for one already-NFC canonical root. TASK-022 adds no compaction or deletion
  policy.
- `RuntimeKind::Live` is structural domain meaning only. Only the durable
  adapter may return a planned Live semantic receipt, and only after commit and
  distinct database/schema/checkpoint evidence.
- Contracts 1.9 and Ports 1.4 do not change. Real Windows/Git inspection,
  filesystem identity acquisition, worktree/change-scope evidence, activation,
  providers/products, and protected release remain outside Project Registry
  1.2.

### `writer-lease` 1.0

- Mission: own project lease, fencing, daemon-instance/epoch validation,
  runtime-admission semantics, suspect-holder, revoke, and reacquire semantics.
- Non-goals: worktree/Git operations, policy decisions, process killing,
  physical tables, or direct provider invocation.
- Owned data: legal lease states, holder identity, fencing-token lifecycle,
  exact command/transition receipts, aggregate replay, and recovery-evidence
  meaning.
- Public contracts: pure transition planning, complete untrusted aggregate
  verification, acquire, heartbeat, inspect, mark suspect, prove
  death/replaced epoch, revoke, release, and deterministic fake composition.
- Invariants:
  1. one active product writer per project;
  2. counters/epochs/revisions are positive, monotonic, non-wrapping signed
     `BIGINT` values;
  3. every daemon-authorized durable mutation validates active daemon
     instance/epoch and runtime-admission mode; product writes also validate
     current fencing token;
  4. expiry alone never proves holder death;
  5. revoked/released tokens are never reused;
  6. exact command retry precedes stale-head denial and changed command
     content rejects permanently;
  7. `DRAINING` denies heartbeat, while `CANARY`/`STOPPED` deny all
     user-project lease transitions.
- Allowed dependencies for the pure owner: contracts, cjson mechanics, exact
  timestamp parsing. PostgreSQL clock/persistence and process-identity
  observation enter only through later adapters and explicit evidence.
- Forbidden dependencies: Git/worktree mutation, provider adapters, policy
  ownership, arbitrary SQL.
- Required TASK-014 gates: pure overflow/non-reuse, exact retry,
  acquire/heartbeat/suspect/release/revoke/reacquire, full admission matrix,
  PID/start identity, newer-epoch recovery, aggregate replay/corruption, and
  Policy receipt/current-head substitution.
- Deferred Step 6 gates: PostgreSQL concurrency/clock/restart, stale live
  connection, and same-transaction fencing for every durable mutation family.

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

TASK-015 activates the pure/fake semantic subset before OS, PostgreSQL,
OpenClaw, Review Runtime, or Guardian integration:

- complete typed subject/challenge/proof hash domains and safe nonce
  commitments;
- globally non-reusable nonce binding, exact applied/denied command retry,
  raw aggregate replay, receipt-chain/high-water, and trusted checkpoint;
- responsible-user/OS and protected-guardian/trust-root lane separation;
- explicit injected time with expiry-exclusive current-head lookup;
- normal claim precondition plus protected pending-claim material, with no
  general protected consume command;
- fixed-producer fake receipt/current-head composition with Policy 2.6.

Live OS/key authentication, database uniqueness/clock/durability/restart,
atomic normal/protected claim, OpenClaw approval IPC, Review Runtime authority,
and Guardian activation remain open.

### `postgres-store` 1.0

- Mission: first own the typed project-scoped physical transaction boundary and
  deterministic zero-I/O conformance fake; later versioned amendments add
  PostgreSQL migrations/runtime admission and one domain repository at a time.
- Non-goals in TASK-018: policy/domain decisions, Git/provider/product I/O,
  arbitrary SQL/row maps, secret storage, database connections, migration
  execution, durable/live claims, or acting as the application orchestrator.
- Owned data in TASK-018: Store transaction envelope, closed repository owner,
  daemon authority/physical heads, request/transaction/receipt hash subjects,
  physical retry identity, terminal fake receipts, bounded fake maps, and
  future explicit migration-manifest ownership. The `control`, `memory`, and
  rebuildable `readmodel` schemas remain the later physical target approved by
  ADR-005.
- Public contracts in TASK-018: typed transact/current-physical-head only.
  There is no generic repository CRUD, SQL, migration, lease, approval,
  artifact, memory, release, or epoch operation. TASK-019 through TASK-025 add
  those physical mechanisms/repositories under separate tickets and versioned
  amendments while consuming each domain owner's public planner/verifier.
- Invariants:
  1. one transaction binds exactly one project/snapshot/closed-owner/aggregate;
  2. exact transaction retry precedes mutable checks and changed ID reuse never
     reveals a prior receipt or changes fake state;
  3. independently retained daemon authority and physical head must match;
  4. applied physical head and terminal receipt appear together or neither;
  5. after-apply response loss is unknown until exact retry reconciles it;
  6. fake receipts are always explicitly non-durable and non-authoritative for
     domain transitions;
  7. no SQL/schema/table/path/arbitrary-record escape hatch exists;
  8. later live migrations are explicit-manifest/checksum/version/compatibility
     checked and never directory-auto-discovered.
- Allowed dependencies in TASK-018: Contracts 1.8, Ports 1.3, cjson, and Rust
  standard-library in-memory collections/errors.
- Forbidden dependencies in TASK-018: PostgreSQL drivers, domain crates,
  OpenClaw, Codex, Graphify, Hermes, Git, product repositories, filesystem,
  network, process, environment, clock, randomness, and credentials.
- Required TASK-018 gates: typed scope/authority/head/digest matrix, atomic
  apply/stale/retry/substitution, bounded capacity/serialization, before/after
  fault reconciliation, fake truthfulness, dependency/no-I/O scan, unchanged
  inert migration hash, full verification, and independent reviews. Live
  migration/concurrency/restart/least-privilege/backup gates remain TASK-019+.

TASK-019 activates version 1.1 as a deliberately narrower database foundation:

- pin the synchronous `postgres` driver and SHA-256 implementation exactly;
- retain `0001_bootstrap.sql` byte-for-byte as a manifest-recorded,
  non-executable `SUPERSEDED` draft and add one transaction-control-free first
  executable migration;
- own exact manifest/history/database-identity/schema-compatibility checks,
  transaction-scoped advisory locking, and a separately invoked administrative
  runner;
- establish only generic future physical Store tables plus a singleton
  `STOPPED`/no-leader runtime-admission row;
- require externally provisioned migrator/runtime/guardian/reader roles,
  verifying them with credential-free `NOLOGIN` fixtures only inside the
  disposable cluster;
- make normal runtime verification read-only and prohibit auto-migration,
  self-election, `ACTIVE` promotion, live `ControlStore`, or durable receipts;
- prove behavior in a marker-owned loopback PostgreSQL 17.10 cluster without
  touching the installed service/data directory or a user database.

Contracts 1.8 and Ports 1.3 do not change in TASK-019. TASK-020 and later
repository tickets require another versioned Store amendment before issuing
live/durable transaction evidence.

TASK-020 activates Contracts 1.9, Ports 1.4, and Postgres Store 1.2:

- Store contract v1 remains fake-only while v2 binds exact Live/
  DurablePostgres evidence to database identity and schema manifest/version;
- the driver-free current-head port becomes explicitly mutable for the
  synchronous connection boundary;
- immutable `0003` upgrades only a verified empty exact-v1 history prefix and
  advances compatibility atomically;
- runtime has zero direct physical/terminal table access and exact EXECUTE only
  on the three fixed prepare/finalize/current-head SECURITY DEFINER functions;
- replay precedes admission, while new mutation checks ACTIVE daemon authority
  and the locked physical head in the same client transaction;
- durable evidence is returned only after commit and commit uncertainty
  reconciles only through reconnect plus exact retry.

This amendment remains physical Store-only. TASK-021 through TASK-024 retain
domain repository ownership; Guardian activation and production provisioning
remain later protected boundaries.

TASK-021 activates Task Ledger 2.1 and Postgres Store 1.3 under the already
approved durable-Ledger direction:

- Task Ledger adds one pure Fake/Live vacant-plan-apply surface, verified
  retained command/outbox replay, and complete independently comparable
  checkpoint while preserving its existing event/head/receipt hashes and
  zero-I/O dependency boundary;
- only an appended `EFFECT_INTENT` with audit outcome `RECORDED` derives one
  immutable outbox admission; claim, delivery, provider calls, and live
  resource observations remain later;
- Postgres Store depends one way on Task Ledger's public semantic planner and
  persists the resulting command, optional event/outbox, projection/checkpoint,
  and physical receipt in one bounded transaction;
- exact `0004` advances the global database profile to v3 while the Store v2
  receipt profile remains fixed to the first three immutable manifest entries,
  preserving byte-identical historical replay across later schema expansion;
- runtime reaches the four new Ledger/outbox tables only through five fixed
  Task Ledger functions, and the historical three Store-v2 functions lose
  runtime execution at v3.

This approved amendment closes only AC-03/04 after direct PostgreSQL evidence.
It does not authorize another domain repository, outbox effect execution,
activation, production provisioning, provider/product work, or release.

TASK-022 governs Project Registry 1.2 and Postgres Store 1.4 under the same
approved one-domain-repository-at-a-time direction and the existing user
directive to continue bounded local implementation through MVP-3:

- Project Registry remains the sole semantic owner and supplies the pure
  runtime-aware global planner/checkpoint/replay boundary described above.
- Postgres Store gains the first and only explicit global persistence exception:
  `PostgresProjectRegistry`. Generic `StoreScope`, `ControlStore`, Store heads,
  and Store receipts remain strictly project/snapshot-scoped. The adapter never
  fabricates a sentinel project/snapshot or a Store receipt for global Registry
  evidence.
- Dependency direction is one-way
  `lattice-postgres-store -> lattice-project-registry`; the Registry domain
  remains I/O-free, no concrete adapter calls another adapter, and Contracts
  1.9 plus Ports 1.4 remain unchanged.
- Exact transaction-control-free `0005_project_registry_repository.sql`
  advances only Fresh or an exact accepted v1/v2/v3 source to global schema v4.
  Migrations `0001` through `0004` remain byte-identical, and a non-empty v3
  source must be `STOPPED` with no leader.
- Schema v4 adds exactly five normalized authoritative Registry tables:
  `control.project_registry_state`,
  `control.project_registry_observations` for immutable complete observations,
  `control.project_registry_projects`, `control.project_registry_commands`, and
  `control.project_registry_identity_reservations`. Fixed columns reconstruct
  complete typed domain input/evidence; authoritative Registry content is not
  JSON, an arbitrary map, an opaque canonical blob, or caller-defined SQL.
- One singleton Registry state row is the global serialization/checkpoint point.
  Every new command uses one bounded `SERIALIZABLE` transaction, exact replay/
  changed-ID classification before mutable admission, the exact ACTIVE daemon
  authority and locked base checkpoint, current-transaction stage provenance,
  and one all-or-none final checkpoint publication. Partial or orphaned rows
  fail closed and are never silently repaired.
- The adapter reconstructs the singleton with
  `RegistryCheckpoint::from_retained` and must use
  `verify_untrusted_registry_snapshot_against_checkpoint`; the plain verifier
  alone cannot establish currentness or reject a self-consistent older prefix.
- Domain evidence follows the normative acyclic order: checkpoint command core,
  canonical logical-state bytes, result checkpoint, and record set. Only then
  may the adapter derive transaction and persistence receipt digests. Physical
  row layout and adapter fields never feed domain logical bytes or checkpoints.
- The schema-v4 runtime surface is exactly three `store_*_v4`, five
  `task_ledger_*_v2`, and nine `project_registry_*_v1` fixed functions. The nine
  Registry roles are prepare; reads of state, observations, projects, commands,
  and reservations; command/immutable-observation staging; changed-project/
  reservation staging; and final checkpoint publication. Historical functions
  remain immutable catalog evidence without runtime EXECUTE.
- Every new function is forward-global-profile-bound, exact-signature,
  migrator-owned `SECURITY DEFINER`, schema-qualified, dynamic-SQL-free,
  non-leakproof, parallel-unsafe, row-security-on, safe-search-path, and bounded
  by `lock_timeout = 5s` plus `statement_timeout = 30s`. Runtime retains zero
  direct SELECT/DML on protected tables.
- Schema v4 preserves Store-v2 receipt profile 2 and its first-three-entry
  manifest commitment. Historical Store receipts and Task Ledger domain
  receipts/checkpoints replay byte-identically; Store-v4 and Ledger-v2 successor
  functions bind the caller's constructor-frozen current global profile.
- Fresh schema v4 seeds one Live vacant singleton with high-water/counts zero,
  the exact 103-byte logical-state accounting, and frozen Live checkpoint
  digest; command rows then remain strict `1..N`.
- A Registry receipt is returned only after commit and exact database/schema/
  checkpoint evidence. Only commit failure with no database response is
  outcome-unknown and poisons the adapter; reconciliation requires a new client
  plus the exact request. Explicit responses remain known outcomes and bounded
  serialization/deadlock retries occur only before uncertainty.

This amendment removes only the Postgres Store non-goal that previously barred
Project Registry persistence. It does not authorize Writer Lease, Approval,
Artifact, Codebase Memory, outbox delivery, live Git inspection, activation,
production provisioning, provider/product work, publication, deployment, or
release. It preserves the original approval and continuation record below; it
does not create a broader approval.

### `artifact-store` 1.0

- Mission: own project-scoped content-object, immutable provenance reference,
  byte-verification/aggregate-quota, generation/availability, exact-command,
  replay/currentness, typed reference authority, retention,
  delete-claim/unknown-outcome reconciliation, and safe-sweep semantics.
- Non-goals: deciding artifact meaning/trust, authenticating a producer,
  product source mutation, policy, general file storage, PostgreSQL mechanics,
  or public filesystem deletion.
- Owned data: `(project_id, sha256)` object identity, positive generation/
  revision, reference/provenance manifest hashes, reference-set/retention/
  availability lifecycle, receipts, replay/checkpoint, and sweep plans.
- Public contracts: publish verified bytes plus initial reference, retain/
  release an exact reference, open verified fake read, query an independent
  available head, plan exact eligible sweep, replay, and checkpoint.
- Invariants:
  1. equal bytes deduplicate only inside one project and current generation;
  2. exact bytes/length/digest plus manifest/object/reference/task/project/
     staging quotas pass before publication or reference mutation;
  3. complete immutable reference manifests preserve provenance without
     granting producer trust or other module authority;
  4. exact retry precedes stale/time checks and denial makes no partial change;
  5. initial publication/reference, retain/release, and read claims require a
     typed fixed-owner receipt plus independently queried current owner head
     bound to exact action/scope/generation;
  6. reference release is terminal and atomically updates checked quotas plus
     the complete current head;
  7. delete claim requires internally recomputed zero references/quota,
     expired retention/grace, exact generation/head, database time,
     daemon/root binding, and typed sweep authority/current head;
  8. `DELETE_CLAIMED` uses one exact token, blocks retain/normal read, and
     reaches deleted, verified no-effect available, or reconciliation-required;
     unknown never guesses success/safety;
  9. delete-claimed/reconciliation/orphan bytes remain worst-case quota until
     verified deletion/publication/cleanup; unknown never releases capacity;
  10. read expiry remains suspect/delete-blocking until holder reconciliation;
  11. reintroduction after deletion increments generation without wrap;
  12. raw bytes never enter metadata receipts/snapshots/errors/`Debug`;
  13. the pure owner performs no I/O or real deletion.
- Allowed dependencies: Contracts 1.7, cjson, exact SHA-256 and canonical-time
  mechanics.
- Forbidden dependencies: ports/adapters, provider-specific semantics, Git,
  filesystem/database/process/network I/O, policy, product repositories, or
  arbitrary deletion.
- Required gates: bounds/digest/quota mismatch, same-project duplicate,
  cross-project/generation/reference/owner-action substitution, exact retry,
  retention race, claim token, unknown/reconciliation, replay/rollback,
  provider non-authority, dependency/no-I/O inspection.
- TASK-016 activation: the deterministic in-memory fake proves pure semantics
  only. PostgreSQL metadata/reference durability and owned-root filesystem
  stage/flush/rename/read/link/sweep behavior remain later AC-19 gates.

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
lattice-task-domain
  -> lattice-contracts
  -> lattice-cjson
  -> exact time 0.3.54 parsing/formatting only
lattice-policy
  -> lattice-task-domain
  -> lattice-contracts

lattice-project-registry
  -> lattice-contracts
  -> lattice-cjson for Registry-owned request/receipt bytes only

lattice-task-ledger
  -> lattice-contracts
  -> lattice-cjson for Ledger-owned request/event/head/receipt/resource bytes
  -> exact time 0.3.54 parsing/formatting only

lattice-postgres-store
  -> lattice-ports
  -> lattice-contracts
  -> lattice-cjson
  -> lattice-task-ledger public planner/verifier
  -> lattice-project-registry public planner/verifier

lattice-orchestrator
  -> lattice-contracts
  -> lattice-policy
  -> lattice-ports

lattice-cli (bootstrap v1 only)
  -> lattice-core-bootstrap inert manifest

workspace-git/project-registry/writer-lease
codebase-memory/approval-verifier
codex-adapter/review-runtime/graphify-adapter/hermes-adapter
  -> lattice-ports
  -> lattice-contracts

artifact-store
  -> lattice-contracts
  -> lattice-cjson
  -> exact SHA-256 and canonical-time mechanics only

future postgres-store artifact repository
  -> lattice-artifact-store public planner/verifier
  -> lattice-contracts

approval-verifier/codebase-memory/self-upgrade-guardian
  -> lattice-cjson for byte mechanics only

latticed -> lattice-orchestrator + concrete adapters

self-upgrade-guardian
  -> narrow release/daemon-epoch/status ports
  -> approval-verifier
```

- Cross-module mutable data has one owner.
- Adapters do not call each other.
- Postgres Store's dependency on the pure Task Ledger and Project Registry
  planner/verifier APIs is a one-way persistence-adapter dependency, not an
  adapter-to-adapter call or transfer of domain legality.
- The Orchestrator owns workflow order; Task Ledger owns event meaning and
  mutable resource-counter semantics; Project Registry owns repository
  identity; Writer Lease owns fencing/epoch semantics; Workspace Git owns
  merge-readiness analysis; Scope Check owns changed-path classification;
  Codebase Memory owns memory transitions; Artifact Store owns object/
  reference/quota/delete lifecycle semantics; the filesystem adapter owns
  physical bytes and exact path mechanics; the PostgreSQL adapter owns
  physical persistence/transaction mechanics only; Policy owns
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

The original module-direction approval did not by itself authorize
implementation. The later continuation and MVP-3 execution directives recorded
below authorize bounded, reversible local implementation, exact dependencies,
disposable database verification, and exact-version local capability
preflights one ticket at a time. Protected actions remain separately gated.

## Approval Record

- Decision: approved
- Date: 2026-07-29
- Approver: responsible user
- Evidence: direct reply `好 開始執行`
- Authorized implementation scope: the bounded, reversible local tickets in
  PLANS.md through MVP-3, including exact dependency setup, disposable
  PostgreSQL verification, local component activation/preflight, and automatic
  routine project review submission.
- Continuation evidence: direct reply `繼續其他部分` authorizes TASK-009's
  local contracts/ports slice; the later direct instruction to execute through
  MVP-3 authorizes subsequent bounded tickets without repeated routine prompts.
- Still protected: account or credential changes, payment, irreversible
  deletion or destructive migration, security-control disablement, public
  publication/exposure/deployment, and primary-branch merge.
