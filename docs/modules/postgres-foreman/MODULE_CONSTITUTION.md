---
module_id: postgres-foreman
name: LATTICE PostgreSQL Foreman Execution Adapter
version: 2.0
status: active
owner: LATTICE maintainers
last_reviewed: 2026-08-31
---

## Mission

Own the explicit same-database `foreman-execution/v1` PostgreSQL extension,
its exact Store-v7 base or Store-v8 rebound database binding, catalog and ACL
verification, and the
durable persistence/replay mechanics for managed worker child evidence and
recoverable capacity-wait reservations, including one bounded physical
Artifact Store outbox per task. Version 1.4 also owns the immutable pre-
successor promotion intent, one bounded rebuttable preparation observation,
Ledger-sequence-derived Artifact replay order, bounded discovery of committed
unpromoted general intake, and same-transaction provider-dispatch current-
authority admission. Task Ledger remains the workflow and task-state authority.
Version 1.5 additionally owns the physical, terminal-equivalent attempt closure
that binds an immutable retained worker blocker to a separate exact
no-provider-effect reconciliation proof without inventing a provider terminal.
Version 1.6 adds one fixed session-scoped serialization guard so the restart
Writer-blocker predicate and existing recoverable Artifact outbox cannot race
terminal, verification, or closure writers using the shared advisory key.
Version 1.7 durably binds every worker observation to the exact App Server
session/home/config identity digest. Identity or generation may rotate only on
an explicit `RECONCILED` observation; the new pair is then immutable again.
Version 1.8 adds Approval-owner snapshot/checkpoint persistence behind a
migrator-only boundary, denies verified-approval self-attestation by the
general Runtime role, and independently recomputes every admitted managed
Artifact content and domain-separated descriptor digest from exact JSON bytes
and closed metadata.
Version 1.9 owns the bounded typed execution-environment descriptor/ref row
attached to each managed attempt and its exact replay/fresh-process mechanics.
Version 2.0 preserves the v1 extension schema and Store-v7 installation row,
then adds one owner-controlled Store-v8 rebind that updates only the singleton
global binding and appends exact ledger ordinal 2 `REBOUND`.

## Non-Goals

- Store or infer Task Domain state, dependency readiness, authorization,
  completion, merge, deployment, publication, payment, or deletion authority.
- Interpret objective text, prompts, commands, paths, project rules, models,
  retries, verification results, approvals, or artifact bytes.
- Modify the global Store manifest or migrations, create another database, run
  Codex, supervise processes, schedule work, or install during normal startup.
- Replace Task Ledger, Approval Verifier, Artifact Store, Writer Lease,
  Foreman State, Orchestrator, or the Codex App Server connector.

## Owned Data

- Exact extension identity and append-only binding ledger: ordinal 1 installs
  against Store v7; ordinal 2 alone rebinds the same extension/database
  identity to exact current Store v8.
- Immutable promotion intent and promotion, worker-attempt, lifecycle
  observation, verification, artifact-reference, and approval-evidence child
  rows. The intent pins the clean Git base/spec/budget before any successor
  Task Ledger effect and grants no execution authority.
- Every lifecycle observation stores the nonzero digest of its exact App Server
  session/home/config identity next to its generation. The digest is bounded
  evidence, not a credential, path, account identity, or execution authority.
- One latest bounded preparation observation per formal intake, limited to a
  typed dirty/currentness blocker or `CLEARED`. It is rebuttable evidence, not
  a task state or queue.
- At most one exact pending worker claim per task. This is a physical
  reservation of an existing Task Ledger worker-attempt event, not an active
  attempt or task-state transition; successful claim consumes it atomically.
- At most one exact staged artifact reference per task, containing the
  owner-verified evidence plus its immutable planned Task Ledger request/link
  until the Ledger event and child row are both durable. It has no task phase.
- Fixed PostgreSQL functions, advisory-lock ordering, capacity constraints,
  exact-retry/substitution rejection, and physical replay queries.
- One immutable attempt-closure row per attempt. A retained worker ambiguity
  closure references both the original blocker descriptor and a distinct typed
  no-provider-effect proof descriptor; neither Artifact Store object is
  rewritten or deleted by this module.
- One exact Approval-owner canonical snapshot plus checkpoint commitment for a
  verified-approval acceptance fixture. It is owner state beneath the Approval
  Verifier contract, not a second approval engine. Task execution-authority
  rows retain only the exact snapshot digest/reference.
- One canonical secret-free execution-environment descriptor per worker
  attempt, independently addressable by its domain-separated digest and bound
  to the exact attempt packet/ref. The descriptor stores only closed WSL,
  canonical path, toolchain, credential-authority-summary, and execution-domain
  identity fields; it is not process, credential, filesystem, or task authority.

The module owns only physical persistence legality. The semantic owner must
first produce the exact Task Ledger append plan. Artifact staging may retain
that exact plan before the Ledger write, but no child row may exist before its
exact Task Ledger event. Artifact Store owns evidence bytes and provenance
semantics; Approval Verifier owns approval currentness and meaning.

## Public Contracts

- Embed and verify only `db/extensions/foreman-execution/v1.sql`; never scan a
  migration directory or extend the global Store manifest.
- Install or rebind only through an explicit migrator-owned runner after exact
  Store schema/manifest, database name, UUID, and derived database identity
  validation. Fresh install accepts only Store v7; the fixed successor accepts
  only the exact Store-v7 base on exact current Store v8. Exact rebound v1 is a
  verified no-op; partial, colliding, substituted, future, or drifted profiles
  fail closed.
- Give normal runtime only fixed `SECURITY DEFINER` functions and schema
  `USAGE`; runtime has no direct table, sequence, DDL, or install privilege.
- Deny the verified-approval ingress to the general Runtime session user before
  any lookup or insert. Only the controlled migrator/Approval-owner acceptance
  boundary may persist an owner snapshot; without a separately authenticated
  production Approval-owner connector, Runtime cannot self-attest approval and
  that execution lane remains fail closed.
- Bind each subordinate row to the exact Task Ledger stream, event sequence,
  event digest, command ID, request digest, subject digest, and fixed action ID.
- Promotion is unique by public `task_ref`. Reservation binds one exact
  Task Ledger worker-attempt event, immutable maximum-attempt budget, attempt
  payload, and promotion without consuming worker capacity.
- Before any successor admission, persist or replay exactly one clean-source
  promotion intent bound to the formal intake event, project receipt, base,
  successor/spec/approval/budget/verification digests and deadline. A second
  matching-lineage successor is ambiguous and fails closed.
- Preparation observations and promotion intents reference only the retained
  Store-v7-owned unique submission stream and global event-digest keys, which
  remain unchanged in Store v8. Every
  record replay and read independently rejoins the exact stream/event pair to
  the same intake task/project/snapshot/receipt; independently valid keys from
  different events are corrupt lineage and raise instead of appearing absent.
- Claim is serialized by one fixed transaction advisory lock, admits at most
  four globally active attempts and one per task, requires monotonic attempts
  and Writer fences, and atomically moves the exact pending reservation to the
  unique `(task_ref, attempt)` active row. A capacity rejection retains the
  reservation for deterministic restart.
- Before a new provider-dispatch claim, use the same transaction and database
  clock to match the attempt's exact persisted execution-authority digest to
  Approval evidence, task/successor/spec/subject/budget/capability/source
  fields and validity interval, and to the current active non-drifted
  user-project Registry snapshot/receipt. Exact immutable claim replay is
  historical replay and precedes these currentness gates.
- Exact retries return replay. Any reuse with changed input, event linkage,
  thread/turn identity, or successor attempt fails closed.
- Environment persistence canonicalizes and recomputes the complete descriptor
  digest at the database boundary before claim. The packet ref and attempt row
  must name the same nonzero descriptor. Exact retry returns the same bytes;
  descriptor/ref, WSL distribution/version, canonical locator, launcher,
  Node/npm/Git/supervisor/sandbox/Rust toolchain, credential-authority kind or
  digest, and execution-domain substitution all fail closed. Restart and
  reconcile reload this row before returning a usable attempt projection.
- Observation exact replay binds the App Server identity digest. Within one
  attempt the latest identity/generation pair is stable; only a durable
  `RECONCILED` observation may select a replacement pair, after which later
  observations must bind that replacement exactly.
- Terminal evidence is append-only. A repair attempt normally follows a proven
  terminal predecessor. The sole non-terminal exception is an owner-validated
  retained worker closure backed by a distinct exact no-provider-effect proof;
  it is terminal-equivalent only for capacity and retry-predecessor admission,
  never a provider terminal, verification, completion, or Writer-release
  authority. Every repair remains bounded by the caller-supplied immutable
  maximum attempt count, never exceeding three in extension v1.
- Retained worker closure, worker reservation/claim, provider dispatch, and
  observation append share the fixed transaction advisory lock. Closure must
  prove the exact attempt/fence, immutable blocker, proof, dispatch claims, and
  observation shape. Possible provider effect rejects closure; a new
  observation after closure rejects. Exact replay may only reproduce an
  already durable identical row.
- Restart Writer-blocker recording acquires only the fixed session guard,
  reloads closure/verification/terminal truth while that guard is held, and
  releases it after the existing independently durable Artifact outbox
  completes. Terminal, verification, and closure functions take the matching
  transaction advisory key. The guard adds no blocker semantics or second
  evidence store.
- Restart discovery returns bounded, ordered, typed rows that distinguish
  committed unpromoted general intake, promoted work with no attempt, capacity
  wait, active reconciliation, and terminal work awaiting verification. The
  unpromoted candidate comes only from the formal Task Ledger ingress/envelope/
  stream plus current Project Registry, excludes existing promotion, is
  keyset-paged, and carries no objective text. Replay marks a pending worker
  event as `PENDING_CLAIM`, never as retained active work.
- Artifact references retain the existing one-MiB per-object bound and are
  atomically limited across retained plus staged objects to 64 objects/eight
  MiB per attempt and 192 objects/24 MiB per task. Exact replay remains legal
  at the limit; a new object is rejected by a closed quota code without adding
  a replay record.
- Artifact persistence is `stage -> Task Ledger append -> atomic finalize`.
  Stage replay binds every evidence, expected-head, event, command, request,
  correlation, and occurred-at field. Restart recovers at most that one exact
  row before full reference projection; changed or stale intent fails closed.
- Artifact staging accepts only exact UTF-8 `application/json`, scans every
  admitted byte sequence for recognized secrets and credential URLs, recomputes
  the raw content SHA, reconstructs the complete canonical descriptor bytes
  from all closed metadata, and recomputes the exact `lattice-hash-1`
  domain-separated descriptor digest. Media relabeling, caller-selected digest,
  noncanonical descriptor, or metadata substitution is rejected before insert.
- Artifact outbox recovery runs only from the managed owner/supervisor
  repository path. Read-only status may validate a staged row in memory but
  never append its Ledger event, finalize it, or admit a provider effect.
- Artifact replay ordinal is the exact owner Task Ledger event sequence, not a
  per-attempt constant. Multiple distinct artifacts therefore replay in one
  deterministic order; duplicate, reordered, or substituted ordinals fail.

## Invariants

1. PostgreSQL is the only durable truth, but Task Ledger is the sole task
   workflow authority; no table or function in this module contains
   `task_state` or performs a Task Domain transition.
2. Store migrations `0001` through `0009` remain immutable; current Store v8
   appends only its reviewed runtime successor.
3. Extension identity always agrees with either the exact Store-v7 base or
   exact current Store-v8 rebound manifest, database UUID, and database
   identity derived from the explicit target name/run ID. Runtime accepts only
   the rebound identity.
4. No child row exists without its exact immutable Task Ledger event, and one
   event cannot be substituted across child records. A staged artifact is not
   a child row and cannot finalize until that event is independently verified.
5. Attempt number, Writer fence, event ordinal, and observation ordinal are
   positive and monotonic within their declared scope.
6. Allowed models are exactly `gpt-5.6-luna`, `gpt-5.6-terra`, and
   `gpt-5.6-sol`; persistence never silently substitutes a model.
7. Capacity is released only by an exact terminal observation or by the one
   owner-validated retained worker closure with a separate exact
   no-provider-effect proof. The latter is not a provider terminal. Elapsed
   time, a blocker alone, process exit, commit, exit code, or verification row
   never releases capacity.
8. Persisted text is bounded, identifier-shaped, and content-free. Full
   prompts, objective text, secrets, raw output, commands, and remote URLs are
   not accepted by this adapter contract. The sole filesystem-path exception
   is the exact canonical Linux/typed locator set inside the versioned
   execution-environment descriptor; those paths are identity only and cannot
   be selected by a caller or interpreted as a command.
9. A pending claim is restart-discoverable but never counts as active capacity
   and cannot own observations, artifacts, verification, a Codex thread, or a
   turn. Only its exact atomic promotion to `worker_attempts` permits launch.
10. Artifact count and byte totals include retained and staged evidence and are
    checked under the same transaction lock as staging/finalization, so
    concurrent writers cannot exceed per-attempt or per-task evidence quotas.
11. The sole pre-start terminal kind is `PRESTART_TERMINAL_FAILED`, bound to
    the exact accepted thread/turn and `FAILED`; it releases capacity without
    ever implying `TURN_STARTED` or `EXECUTING`.
12. A new provider claim is indivisible from current Approval-evidence,
    database-time, task/spec/budget and Project Registry revalidation. Expired,
    substituted, inactive, pending-observation, drifted, or changed-snapshot
    authority creates no claim and no external effect.
13. Unpromoted-intake discovery is a read-only locator over formal Ledger and
    Registry truth, not a shadow queue or second lifecycle. Once an exact
    promotion exists, that task cannot remain a draft candidate.
14. Read-only status and replay never recover a staged Artifact outbox. Only
    the owner recovery path may execute the exact staged plan and finalize it.
15. Preparation observations are fixed-size, intake-bound and replace only
    their prior observation with a new digest/generation. They cannot grant
    execution or replace the successor Task state.
16. Promotion intent accepts only `source_clean=true`; a restart reuses its
    exact base and successor identity without observing mutable Git HEAD.
17. Every Artifact replay ordinal equals its immutable Task Ledger event
    sequence, so two artifacts in one attempt cannot collide.
18. A restart Writer blocker cannot pass its final durable predicate while a
    terminal, verification, or closure write commits concurrently. The
    session guard spans `stage -> Ledger append -> finalize` without merging
    those crash-recoverable commits into one transaction, and is always
    released or abandoned with its owning database session.
19. Worker observation persistence never infers App Server identity from a
    generation counter. It stores and exact-replays the nonzero identity digest,
    rejects identity/generation drift outside `RECONCILED`, and locks the most
    recently reconciled pair for every later observation in that attempt.
20. A Runtime database credential cannot write or activate
    `VERIFIED_APPROVAL`, even with well-formed owner bytes and matching caller-
    computed hashes. Closed-policy evidence remains the only Phase-4 product
    execution ingress until a distinct Approval-owner connector/role exists.
21. Persisted managed Artifact bytes and descriptor identity are both
    independently recomputed at the database boundary; content hash, media
    label, descriptor bytes, descriptor digest, or secret-bearing payload
    substitution cannot become a retained reference.
22. Every claimed WSL2 attempt has exactly one independently queryable
    execution-environment descriptor whose recomputed digest equals both its
    stored ref and the attempt packet ref. Fresh-process reconstruction,
    restart, retry, reconnect, and reconcile never sample a replacement value;
    any field or digest drift blocks before another provider/verifier effect.

## Allowed Dependencies

- `lattice-contracts` for shared content-digest values.
- `lattice-task-ledger` for verified write records and untrusted replay rows.
- `lattice-foreman-state` for the owner-validated immutable worker budget and
  exact reconstruction after restart.
- `lattice-artifact-store` and `lattice-approval-verifier` for their verified
  subordinate records and untrusted replay forms; semantic verification stays
  in those owner crates.
- Exact synchronous PostgreSQL and SHA-256 crates already locked by the
  workspace.

## Forbidden Dependencies

- `lattice-postgres-store`, any other PostgreSQL adapter, Orchestrator, Codex,
  Writer Lease,
  project repositories, provider/model SDKs, dynamic SQL/migration discovery,
  environment/credential loaders, shells, or runtime composition.
- Adapter-to-adapter calls, semantic lifecycle planners, caller-provided SQL,
  or objective-derived values.

## Failure, Compatibility, And Migration

Version 1.9 installation accepts only exact Store schema v7 plus its frozen
manifest.
Fresh install and exact no-op are the only administrative successes. Partial,
extra, ACL-drifted, cross-database, replaced-function, event-substituted, or
ambiguous state fails closed without repair, downgrade, or automatic startup
migration.
The 1.9 environment binding, 1.8 Approval/artifact hardening, 1.7 observation-
identity contract, and 1.6 guard change the reviewed extension SQL/profile
while preserving Store v7. Pre-release disposable 1.6/1.7/1.8 profiles must be rebuilt; an
existing non-disposable older profile is incompatible and blocks adoption
until an explicit versioned migration decision. Runtime never repairs it.

Version 2.0 is that explicit compatibility decision. The fixed owner asset
accepts only the exact one-ledger Store-v7 predecessor while the Store is exact
current v8, updates the singleton binding, and appends ordinal 2 `REBOUND`.
It preserves every managed child row and extension SQL object. Exact retry is
read-only; Runtime rejects the Store-v7 base after Store reaches v8.

## Acceptance Gates

| Gate | Evidence | Owner | Required for merge |
|---|---|---|---|
| Embedded identity | exact byte/hash/manifest contract tests | Engineering | yes |
| Install/no-op/rollback | disposable PostgreSQL 17 fresh/repeat/partial/collision tests | Engineering | yes |
| Catalog and ACL | exact owned-object/function/grant verification plus runtime table denial | Security review | yes |
| Claim safety | durable reservation, concurrent global-four, per-task-one, monotonic attempt/fence, exact retry and substitution tests | Engineering | yes |
| Retained no-effect closure | same-lock claim/closure race, immutable blocker plus distinct proof, possible-effect rejection, post-closure observation denial, exact replay/tamper, capacity release, and bounded retry tests | Engineering and security review | yes |
| Restart Writer-blocker serialization | session guard spans durable predicate reload and recoverable Artifact outbox; matching terminal/verification/closure transaction locks; guard release/error/crash analysis | Engineering and security review | yes |
| App Server observation identity | table/function/adapter round-trip, exact-replay substitution, non-reconciled drift rejection, reconciled rotation, and fresh-process reconstruction tests | Engineering and security review | yes |
| Execution environment binding | canonical descriptor/ref insert and independent query, packet/attempt binding, exact replay, field/ref/toolchain/path substitution denial, and fresh-process restart/reconcile equality against disposable PostgreSQL 17 | Engineering, architecture, and security review | yes |
| Event lineage | missing/substituted Task Ledger event and action rejection tests | Integration review | yes |
| Replay | fresh process distinguishes promoted, capacity-wait, active and terminal work, then replays identical evidence without another attempt event | Engineering | yes |
| Draft restart discovery | committed-intake-before-promotion crash, bounded keyset replay, current Registry substitution, exact promotion disappearance, and no duplicate successor | Engineering and security review | yes |
| Provider authority admission | same-transaction expiry/task/spec/budget/receipt and Registry-currentness rejection, TOCTOU resistance, plus exact immutable replay after expiry | Security review | yes |
| Approval owner isolation | migrator-owned snapshot/checkpoint physical restart and tamper replay plus direct Runtime-role verified-ingress and table-access denial | Security review | yes |
| Artifact descriptor ingress | raw-SHA, exact JSON/UTF-8 media, all-byte secret scan, canonical descriptor reconstruction, domain-frame digest, relabel and substitution live negatives | Security review | yes |
| Evidence quota | atomic per-attempt/per-task count and byte limits, exact replay at limit, and closed rejection tests | Engineering | yes |
| Artifact crash recovery | real PostgreSQL stage, Ledger append, process/server restart, exact atomic finalize/replay, substitution rejection, and unchanged Task Replay before finalize | Integration review | yes |
| Global compatibility | immutable Store-v7 installation evidence plus exact Store-v8 singleton rebind and ordinal-2 ledger append; legacy/current retry, partial/cross-pair/future rejection, and fresh-runtime verification | Compatibility and architecture review | yes |
| Dependency direction | Cargo metadata and forbidden-dependency scan | Architecture review | yes |

## Change Policy

Mission, ownership, Store compatibility, SQL/profile identity, table/function
surface, lock/capacity/replay semantics, dependencies, or privileges require a
versioned constitution amendment, SPEC/ADR trace, and architecture/security
review.

## Amendment History

| Version | Date | Decision reference | Summary | Approver |
|---|---|---|---|---|
| 2.0 | 2026-08-31 | ADR-029 managed-foreman deployment repair | Preserve extension-v1 data and Store-v7 install evidence, then append the exact Store-v8 binding successor at ledger ordinal 2 | User-authorized deployment repair |
| 1.9 | 2026-08-28 | SPEC-011 v1.9, ADR-028 WSL2 durable-environment amendment | Persist one canonical secret-free execution-environment descriptor/ref per attempt with database-side digest recomputation, exact replay, substitution denial, and fresh-process restart/reconcile reconstruction | User-authorized Phase 4 WSL2 handoff |
| 1.8 | 2026-08-28 | SPEC-011 v1.8, ADR-028 approval/artifact security review | Persist replayable Approval-owner snapshots only behind the migrator boundary, deny Runtime verified-approval self-attestation, and recompute managed Artifact content/canonical descriptor identity before insert | User-authorized Phase 4 |
| 1.7 | 2026-08-28 | SPEC-011 v1.7, credential-isolation review | Persist exact App Server session/home/config identity digest on every observation and permit identity/generation rotation only through durable reconciliation | User-authorized Phase 4 repair |
| 1.6 | 2026-08-28 | SPEC-011 v1.6, independent Phase 4 recovery review | Serialize restart Writer-blocker durable predicate and Artifact outbox against terminal, verification, and closure writers with one fixed session guard | User-authorized Phase 4 repair |
| 1.5 | 2026-08-28 | SPEC-011 v1.4, ADR-028 amendment | Add owner-atomic retained worker no-effect closure as a terminal-equivalent capacity/retry predecessor without fabricating a provider terminal or Writer authority | Delegated product owner |
| 1.4 | 2026-08-27 | SPEC-011 durable-core review | Add clean immutable promotion intent before successor effects, bounded rebuttable preparation observations, ambiguous-successor rejection, and Ledger-sequence Artifact replay ordinals | Delegated product owner |
| 1.2 | 2026-08-27 | SPEC-011, ADR-028 | Add a one-row-per-task staged Artifact Store outbox with exact Ledger-bound restart recovery and retained-plus-staged quota accounting | Delegated product owner |
| 1.3 | 2026-08-27 | SPEC-011, ADR-028 durable-core review | Add formal unpromoted-intake restart discovery, typed pre-start failure, owner-only outbox recovery, and atomic current Approval/Registry provider admission | Delegated product owner |
| 1.1 | 2026-08-27 | SPEC-011, ADR-028 | Add recoverable pending claims, typed restart discovery, replay state, and bounded artifact-reference quotas | Delegated product owner |
| 1.0 | 2026-08-26 | SPEC-011, ADR-028 | Add the subordinate same-database foreman execution extension without changing Store v7 or Task Domain state | Delegated product owner |
