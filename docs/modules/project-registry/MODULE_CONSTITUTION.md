---
module_id: project-registry
name: Project Registry
version: 1.2
status: active
owner: LATTICE maintainers
last_reviewed: 2026-08-03
---

## Mission

Own canonical project/repository identity and the deterministic
register/resolve/suspend/drift/reconcile lifecycle, returning immutable
snapshot-bound authority receipts that other modules can verify without
becoming project-identity owners. Own one complete, bounded, runtime-aware
global checkpoint and command order so every accepted and pending identity is
verified as one catalog without turning persistence into a semantic owner.

## Non-Goals

- Perform filesystem, Git, database, process, network, clock, credential, or
  provider I/O.
- Create Git worktrees, change refs, classify changed paths, decide policy, or
  execute a task.
- Bind a project receipt to a task ID or Task Spec hash.
- Claim fake/in-memory evidence is durable PostgreSQL authority.
- Define the real Windows file-identity or Git packed/loose-ref inspection
  algorithm in TASK-012.
- Perform PostgreSQL migration, catalog, ACL, transaction, retry, locking, or
  persistence-receipt mechanics, or depend on Postgres Store.
- Fabricate a project-scoped `StoreScope`, `ProjectSnapshotId`, or
  `StoreTransactionReceipt` for a global Registry command.
- Compact, delete, silently truncate, or archive retained command history in
  version 1.2.

## Owned Data

- Canonical root and root-identity semantics.
- Repository and filesystem/file identity semantics.
- Registered project class, lifecycle, Registry revision, immutable snapshot
  lineage, accepted observation, and pending drift observation.
- Duplicate-identity, suspension, move/replacement drift, and reconciliation
  transition meaning.
- Registry 1.1 observation, request, authority-receipt, and command-result hash
  subjects. `result_digest` is the terminal semantic command-result
  commitment; Registry 1.1 owns no separate terminal-receipt or record-set hash
  subject.
- The global catalog command ordinal, complete first-seen command history,
  checkpoint command core, accepted/pending identity-reservation projection,
  new Registry 1.2 record-set meaning, exact logical-retained-state accounting,
  and immutable Registry checkpoint.
- The verified retained-state representation and the deterministic vacant,
  plan, apply, export, replay, and verification semantics used by every
  runtime.

Project Registry owns semantic truth. `lattice-contracts` owns only shared
immutable representations. Postgres Store 1.4 owns physical durability and
Registry persistence evidence without owning lifecycle legality, command
outcomes, reservations, revisions, snapshots, or checkpoint meaning.

## Public Contracts

- Construct one vacant verified Registry state for an explicit `Fake` or
  `Live` runtime without I/O.
- Plan exactly one typed Registry command against a verified retained state and
  complete base checkpoint without mutating that state.
- Apply exactly one plan only after rechecking the complete base checkpoint,
  yielding a new verified state, record set, semantic receipt, and result
  checkpoint.
- Export the complete observations, current projects, first-seen command
  records, accepted/pending reservations, accounting, and checkpoint as an
  explicitly untrusted snapshot.
- Reconstruct a separately retained singleton through
  `RegistryCheckpoint::from_retained` without claiming that checkpoint is
  current or that its corresponding rows are self-consistent.
- Use plain `verify_untrusted_registry_snapshot` only to prove an exported
  snapshot's internal self-consistency. Use
  `verify_untrusted_registry_snapshot_against_checkpoint` to additionally
  compare the separately read retained singleton; this is the only verifier a
  durable adapter may use before returning current authority.
- Verify by replaying complete commands in global ordinal order from a vacant
  state and comparing every retained observation, project, reservation,
  command core, count, runtime, record set, receipt, logical byte count, and
  checkpoint before constructing verified retained state.
- Preserve the complete original typed request, semantic `RegistryCommandReceipt`,
  non-zero ordinal, and base/result checkpoints for every first-seen terminal
  command. Version 1.2 exposes no history-deletion or compaction operation.
- Accept an already inspected, immutable `RepositoryObservation`.
- Register one canonical project identity and issue revision/snapshot 1.
- Resolve an exact current observation without rotating its snapshot.
- Detect root, repository, file, primary-ref text, or primary-ref physical
  identity drift and return non-active reconciliation-required authority.
- Suspend an exact current project and return non-active authority.
- Reconcile an exact suspended/drifted head using the matching decision and
  immutable evidence, rotate revision/snapshot, and preserve old receipts.
- Accept only already-NFC canonical command IDs, roots, and primary-ref text;
  hidden normalization never changes a command subject or hash input.
- Reject duplicate project IDs and duplicate accepted or pending
  root/repository/file identities, including aliases represented by the same
  physical identity digest.
- Reserve an accepted pending observation for its owning project until exact
  reconciliation. Another registration or reconciliation cannot front-run
  that reservation.
- Distinguish a zero-mutation terminal `Denied` outcome from a defensive
  state-changing `Blocked` outcome. Registration collisions deny without
  mutation. An authoritative observation that collides with another project's
  identity blocks, rotates the observed project to `SUSPENDED`, and does not
  retain the colliding observation as a reservation.
- Issue idempotent terminal command receipts and reject command-ID subject
  substitution.
- Return the fake owner's exact current head for composition tests while
  explicitly making no durability or producer-authentication claim.
- Expose the exact Registry binding required by future Scope Check composition
  without classifying changed paths.

## Invariants

1. One physical root, repository, or file identity cannot be active under two
   project IDs. Accepted identities take precedence over pending reservations.
   The first non-colliding pending observation reserves its identity for its
   owning project; a colliding observer never receives a second reservation.
2. Project class never changes after registration.
3. Every state mutation advances a non-zero, non-wrapping Registry revision
   and produces a new immutable snapshot; exact resolve changes no project
   revision or authority snapshot.
4. Old snapshots and receipts are immutable evidence but never current
   authority after the head advances.
5. Moved, replaced, suspended, stale, cross-project, or ambiguous identity
   evidence cannot produce `ACTIVE` current authority. If an authoritative
   observation of an `ACTIVE` project collides with another project's accepted
   or pending identity, the observed project advances to a new `SUSPENDED`
   head and the terminal outcome is `Blocked`, not `Denied`.
6. Primary refs are fully qualified local `refs/heads/*`; ref text preserves
   case while physical identity digest comparison catches storage aliases.
7. Same command ID plus same request replays one identical terminal receipt;
   same command ID plus a different request is rejected.
8. Fake Registry receipts are visibly `RuntimeKind::Fake` and cannot claim
   PostgreSQL durability or live filesystem inspection.
9. Command IDs, canonical-root text, and primary-ref text must already be NFC.
   Rejection occurs before mutation; semantically equivalent non-NFC text
   cannot create a distinct canonical hash subject.
10. Receipt hashing uses `lattice-cjson-1`; raw map/debug/JSON text is never a
   hash input. `Denied` and `Blocked` are distinct hashed terminal outcomes.
11. The module performs no I/O and has no hidden clock, randomness, global
    mutable singleton, or product-repository access.
12. The global checkpoint binds explicit runtime/version, one non-negative
    signed-BIGINT-compatible non-wrapping catalog command high-water, every
    current project projection, all accepted and pending reservations, the
    complete first-seen semantic command cores in ordinal order, deterministic
    counts, exact logical-state bytes, the logical state itself, and its
    canonical digest. Vacant high-water is `0`; first-seen records are the
    strict positive sequence `1..N`.
13. Every first-seen terminal command advances the catalog ordinal/checkpoint
    exactly once, including zero-project-mutation `Denied`, state-changing
    `Blocked`, and exact no-project-change observation. Only legal project
    lifecycle mutations advance the separate project Registry revision and
    authority snapshot. Same-command/same-request replay advances neither;
    changed reuse returns no historical receipt.
14. Only vacant construction, verified replay, or checked plan application may
    create verified retained state. Exported observations, projections,
    reservations, commands, accounting, and checkpoints are untrusted input
    until complete comparison succeeds.
15. Missing, extra, reordered, duplicated, injected, or altered command
    history; denial-tail rollback; observation/project/reservation drift;
    count/accounting mismatch; runtime substitution; or checkpoint-chain
    disagreement fails closed and yields no current authority.
16. Version 1.2 admits at most 4,096 current project projections, 65,536
    first-seen terminal command records, 67,108,864 bytes (64 MiB) of
    Registry-owned logical-retained-state canonical bytes, and 131,072 UTF-8 bytes
    (128 KiB) in one already-NFC canonical root. Exact replay and changed-ID
    classification precede capacity checks. A first-seen over-limit request
    fails closed before project mutation, ordinal/checkpoint advance, receipt
    construction, persistence planning, or history truncation.
17. Fake and Live use the same vacant/plan/apply/export/verify semantics. The
    Fake wrapper forces `RuntimeKind::Fake`. Registry 1.1 freezes only
    observation, request, authority-receipt, and command-result literal digest
    vectors; it preserves those and every existing TASK-012 collision/lifecycle
    behavior byte-for-byte. Registry 1.2 adds separate checkpoint and
    record-set vectors.
18. Canonical commitments are constructed only in this acyclic order:
    checkpoint command core; complete logical-retained-state projection and
    byte count; result checkpoint; record set; adapter transaction digest; and
    persistence receipt. The command core contains only ordinal, complete typed
    request, and complete semantic `RegistryCommandReceipt`. Base/result
    checkpoint references, record-set/count/retained-byte fields, database/
    schema evidence, transaction digest, and persistence receipt are excluded.
    Checkpoint references form a verified chain but never feed a checkpoint.
19. The record set binds the command persistence core (command core plus base/
    result checkpoint references), any newly inserted immutable observation,
    an optional current project replacement, and exact ordered reservation
    deletes/inserts. It excludes its own digest and every PostgreSQL/adapter
    field. A physical command-row convenience projection cannot replace any
    domain projection.
20. `lattice.project-registry.logical-retained-state` schema version `1` contains exactly
    `schema_version`, `runtime`, `observations`, `projects`, `commands`, and
    `reservations`. Complete observations are keyed/sorted by digest and
    counted once even when multiply referenced; projects are sorted by Project
    ID and reference observation digests; commands are checkpoint command
    cores in strict ordinal order; reservations are sorted by identity
    dimension, identity digest, status, and Project ID. Optional fields are
    explicit canonical `null`, text is already NFC and counted as encoded
    UTF-8, and unsigned/count values are canonical decimal strings.
21. Retained bytes equal exactly `canonicalize(logical_state).len()`. Hash
    framing, base/result checkpoint references, counts, the retained-byte
    field, checkpoint/record-set digests, SQL row overhead, database/schema
    evidence, transaction fields, and persistence receipts are excluded. The
    exact vacant Live logical state is
    `{"commands":[],"observations":[],"projects":[],"reservations":[],"runtime":"LIVE","schema_version":"1"}`
    at 103 bytes. With high-water/counts zero, the frozen vacant checkpoint
    digests are `22ad9599c05ab384e720b8f1d91bdfbe1262360850602aa0f6b5fd79c1797f4f`
    for Fake and `5bb1f9d9adf7228bef7ea45cc029e79d71761c4dfed3abe086634917db38c173`
    for Live.

## Allowed Dependencies

- Rust standard library.
- `lattice-contracts` 1.9 for shared immutable identity and receipt values.
- `lattice-cjson` 1.0 for canonical receipt/request byte mechanics only.

## Forbidden Dependencies

- `lattice-policy`, `lattice-ports`, Task Domain, Task Ledger, Workspace Git,
  Scope Check, Orchestrator, PostgreSQL store, provider adapters, and guardian.
- Filesystem, Git, database, process, network, clock, randomness, environment,
  credential, model, payment, publication, or deployment libraries.

## Failure, Compatibility, And Migration

Invalid inputs and unknown contract versions fail closed with stable typed
codes. Non-canonical text, revision overflow, stale expected head, mismatched
reconciliation kind, registration/reconciliation duplicate identity, and
command replay substitution return `Denied` or a typed error without mutation.
Catalog ordinal overflow, retained-state limit overflow, malformed or
unverified exported state, command-history/checkpoint disagreement, and runtime
substitution likewise fail closed without exposing current authority.

An authoritative observation collision is deliberately different: retaining
the old `ACTIVE` authority would be unsafe after owner-supplied evidence shows
the identity is not exclusive. It therefore returns `Blocked`, advances the
revision/snapshot, transitions the observed project to `SUSPENDED`, clears its
pending observation, and leaves the other project's accepted or pending
reservation unchanged.

The Fake state remains disposable, but Project Registry 1.2 must reproduce the
frozen Registry 1.1 Fake observation, request, authority-receipt, and
command-result digests plus replay, revision, reservation, and snapshot
behavior byte-for-byte through the shared planner/verifier. Registry 1.2
checkpoint and record-set subjects are new compatibility vectors, not
retroactive Registry 1.1 hash subjects.

Postgres Store 1.4 is the only TASK-022 physical adapter. It must preserve
migrations `0001` through `0004`, historical Store-v2 and Task Ledger receipt/
checkpoint bytes, and new Ledger availability while adding the exact schema-v4
five-table/nine-function Registry profile and distinct global persistence
evidence defined by SPEC-002 v24 and ADR-020. Restart, concurrency,
commit-response loss, partial staging, ACL/profile drift, and retained-row/
checkpoint/persistence corruption must converge or fail closed before any Live
Registry authority is returned. These are adapter acceptance requirements;
this domain module still performs zero I/O and has no PostgreSQL dependency.

## Acceptance Gates

| Gate | Evidence | Owner | Required for merge |
|---|---|---|---|
| Registration and exact resolve | focused deterministic tests | Engineering | yes |
| Duplicate/cross-project isolation | accepted/pending reservation, front-run denial, defensive `Blocked`, and reactivation matrix | Security review | yes |
| Drift/suspend/reconcile | lifecycle, stale-head, and snapshot-lineage tests | Engineering | yes |
| Command idempotency | same/different request replay tests | Engineering | yes |
| Global checkpoint and ordinal | vacant `0`, strict `1..N`, first-seen applied/denied/blocked/read-only order, separate project revision, overflow, and checkpoint-chain tests | Engineering | yes |
| Export and verified retained state | plain self-consistency versus `from_retained` plus `verify_untrusted_registry_snapshot_against_checkpoint`; missing/extra/reordered/duplicate/corrupt/coherent-prefix rollback matrices | Security review | yes |
| Bounded retained state | exact canonical logical-state algorithm, unique observation accounting, 103-byte vacant fixture, 4,096-project, 65,536-command, 64-MiB logical-byte, and 128-KiB root boundary/plus-one tests | Security review | yes |
| Fake golden parity | Registry 1.1 observation/request/authority-receipt/command-result vectors plus new Registry 1.2 checkpoint/record-set vectors and existing lifecycle matrix | Engineering | yes |
| Acyclic commitments | command core -> logical bytes -> result checkpoint -> record set -> adapter transaction/persistence tests; forbidden-field substitution matrix | Architecture review | yes |
| Receipt exactness | canonical digest and field-substitution matrix | Security review | yes |
| No I/O/dependency drift | Cargo tree plus forbidden-reference scan | Architecture review | yes |
| Policy composition boundary | receipt/head substitution tests in Policy | Architecture review | yes |
| Full verification | Rust workspace and preserved Node suite | Engineering | yes |
| Real Windows/Git identity | disposable platform repository fixtures | Future Workspace Git ticket | no |
| TASK-022 PostgreSQL durability | exact schema-v4 five-table/nine-function profile; global transaction, restart, concurrency, commit-uncertainty, partial-stage, corruption, ACL, and Store/Ledger compatibility tests | Postgres Store 1.4 | yes |

## Change Policy

Mission, owned identity semantics, lifecycle, snapshot rotation, receipt
subjects, duplicate/drift rules, dependency direction, or acceptance gates
require a versioned amendment, specification/ADR trace, architecture and
security review, and responsible-user authorization.

## Amendment History

| Version | Date | Decision reference | Summary | Approver |
|---|---|---|---|---|
| 1.0 | 2026-07-29 | SPEC-002 v7, ADR-010, TASK-012 | Pure Project Registry owner contract and deterministic fake receipts | User MVP-3 execution directive |
| 1.1 | 2026-07-29 | SPEC-002 v8, ADR-010 review amendment, TASK-012 | Separate zero-mutation collision denial from defensive authoritative-observation blocking; reserve accepted pending identities and require NFC command subjects | User MVP-3 execution directive |
| 1.2 | 2026-08-03 | SPEC-002 v24, ADR-020, TASK-022 | Add the pure global checkpoint/ordinal and vacant/plan/apply/export/verify boundary, complete retained history, Fake golden parity, fail-closed limits, and the Postgres Store 1.4 durability gate without adding I/O or a PostgreSQL dependency | User MVP-3 execution directive |
