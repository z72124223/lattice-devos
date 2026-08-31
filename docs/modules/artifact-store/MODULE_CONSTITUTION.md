---
module_id: artifact-store
name: Artifact Store
version: 1.1
status: active
owner: LATTICE maintainers
last_reviewed: 2026-08-26
---

## Mission

Own pure, replayable, project-scoped content-object, immutable provenance
reference, byte-verification, aggregate quota, availability, idempotency,
current-head, retention, durable-delete-claim, unknown-outcome reconciliation,
and safe-sweep semantics; issue fixed-owner receipts through a deterministic
non-durable fake; and define the one semantic core later reused by PostgreSQL
metadata and an owned-root filesystem blob adapter.
Version 1.1 additionally owns the small immutable, bounded managed-foreman
evidence object used by the PostgreSQL foreman extension.

## Non-Goals

- Decide whether artifact content is true, safe, extracted, inferred,
  review-approved, accepted memory, approved code, or releasable.
- Authenticate a source producer, provider, reviewer, user, or Guardian.
- Decide Policy, task legality, project lifecycle, writer authority, approval,
  memory promotion, merge, activation, or release.
- Perform filesystem, database, Git, process, network, environment, provider,
  credential, payment, publication, deployment, or product-repository I/O.
- Expose a public filesystem unlink or arbitrary cleanup command.
- Claim fake bytes/receipts are live, durable, OS-contained, or
  PostgreSQL-backed.

## Owned Data

- Project-scoped `(project_id, sha256)` object identity and positive
  signed-BIGINT-compatible generation/revision semantics.
- Immutable object descriptor and per-use reference/provenance manifest hash
  selection.
- Raw byte digest/length verification and fixed per-object/manifest/reference/
  task/project/staging hard limits plus checked quota projections.
- Active-reference set, retention/grace, availability, exact command,
  predecessor receipt chain, raw replay, and trusted checkpoint semantics.
- Fixed `lattice-artifact-store` receipt issuance and independent available
  current-head projection.
- Typed reference-owner retain/release authority binding.
- Exact managed-foreman evidence bytes, closed evidence kind, sanitized
  producer metadata, direct content digest, and separate canonical descriptor
  digest. This object grants no truth, approval, task-state, or completion
  authority.
- A managed-worktree pre-dispatch baseline reuses `GIT_SNAPSHOT` with the
  closed `lattice.managed-worktree-baseline/1.0` payload schema. Its content
  digest is the immutable attempt `worktree_ref`; attempt-specific descriptor
  metadata cannot redefine that baseline.
- Pure exact sweep planning, delete-claim/token, known-failure,
  unknown-outcome/reconciliation, and fake lifecycle application.

Contracts owns shared immutable representation only. PostgreSQL will own
physical serialization and transaction mechanics only. A future filesystem
adapter owns byte staging/read/unlink mechanics only.

## Public Contracts

- Accept one complete immutable artifact binding, descriptor, provenance, and
  initial reference only with a typed owner receipt/current-head pair for
  `PUBLISH_INITIAL_REFERENCE`; never choose or broaden project/task/producer
  scope.
- Require exact Registry snapshot, effect claim, daemon instance/epoch/
  admission, capability-owner receipt/current-head, producer/adapter binary,
  correlation/run/sequence/produced-at/payload, and limit-snapshot bindings.
- Verify declared length and SHA-256 before publishing an object/reference.
- Deduplicate equal bytes only inside one project and one available generation.
- Add one immutable reference to the exact current object generation only from
  a typed fixed-owner receipt plus independently obtained current owner head.
- Release one exact reference terminally from the same typed owner-authority
  pair without erasing provenance/history.
- Query an independent complete available head and perform a digest-verified
  fake read through an explicit active-read claim.
- Plan and claim deletion only from internally recomputed zero references,
  expired retention/grace, exact generation/current head/quota projection,
  explicit observation/database time, current daemon authority, and a typed
  fixed-owner sweep receipt/current-head pair.
- Represent success, verified no-effect failure, and unknown outcome
  separately; require verified reconciliation before leaving an unknown state.
- Return identical terminal receipts for exact retries and reject changed
  command content permanently.
- Export and verify a strict untrusted raw aggregate snapshot.
- Export and compare a validated trusted checkpoint for rollback-sensitive
  restore.
- Construct and strictly replay-verify a maximum-1-MiB managed-foreman evidence
  object while keeping raw bytes out of `Debug` and descriptor bytes.

## Invariants

1. Object identity is project-scoped; no cross-project key, deduplication,
   lifecycle, path, reference set, or existence response is shared.
2. Content digest is SHA-256 of exact bytes. Metadata hashes use distinct
   `lattice-cjson-1` domains; one cannot substitute for the other.
3. Empty bytes are legal. Declared/observed length must be equal. Hard limits
   are 1 GiB/object, 64 KiB/canonical manifest, 65,536 active references/object,
   100,000 objects/task, 1,000,000 references/task, 64 GiB active referenced
   bytes/task, 4 GiB and 8 concurrent staging streams/task, 4,096 read
   claims/object, 65,536 read claims/task, 1,000,000 commands/object,
   5,000,000 commands plus 1 GiB command history/task, 1,000,000 available
   objects/project, 10,000,000 active references/project, 1,000,000 read
   claims/project, 100,000,000 commands plus 64 GiB history/project, 1 TiB
   unique bytes/project, 10,000,000 objects/50,000,000 references/5,000,000
   reads/8 TiB bytes/16 GiB and 64 staging streams/500,000,000 commands and
   256 GiB history/store, 100,000 bundle entries, bundle depth 64, and 256
   UTF-8 bytes per bounded identifier/media/schema/producer field.
   Configuration may only lower them and every command binds an independently
   loaded immutable limit-snapshot digest.
4. Every reference binds project/snapshot/task/revision/spec/attempt/request,
   object/generation, media/schema/bundle bounds, producer/version/runtime/
   binary, adapter/version/binary, invocation/correlation/run/sequence/
   produced-at/payload, capability/input/config/evidence, Registry authority,
   effect-claim, daemon instance/epoch/admission, capability-owner receipt/
   current head, limit snapshot, purpose, and retention fields.
5. A provider may appear only as provenance. Only the fixed Artifact Store
   producer can issue store receipts; those receipts grant no content trust or
   other module authority.
6. User filenames and metadata never determine a storage path.
7. Idempotency key is exactly
   `(project_id, algorithm, content_digest, command_id)`. Exact retry precedes
   stale-head/time checks; changed command content rejects permanently.
8. Applied and denied terminal receipts form one predecessor chain. Every
   durable record retains complete sanitized canonical request source, digest,
   key, and terminal receipt without raw bytes. Aggregate high-water/tail and a
   trusted checkpoint detect denial-tail loss and coherent-prefix rollback.
9. A released reference is terminal and cannot be rebound or made active.
10. Reference addition/release changes revision and full-head equality.
11. `receipt.head()` is structural only. Historical, swept, deleted,
    wrong-generation, fake/live-mixed, or substituted evidence is not current
    available authority.
12. Initial publication/reference, retain, and release never accept caller
    counts, Booleans, producer strings, or bare evidence digests. A typed
    fixed-owner authority receipt/current-head pair binds owner
    record/revision/status, exact action, scope, object, generation, and
    reference. Fake authority is visibly fake; live authority is unavailable
    until its owner contract exists.
13. Quota counters and deltas are owner-computed. Publication/reference/
    read/delete/reintroduction atomically checks object/task/project/store/
    staging projections; a caller cannot assert `within_quota`. Project/store
    bytes count each non-deleted generation once; task bytes count one object
    once when that task has any active reference; reference/read/staging/
    command/history counters count their exact identities or canonical bytes.
14. `DELETE_CLAIMED`, `RECONCILIATION_REQUIRED`, and sealed orphans retain
    worst-case object/byte/staging quota. Object quota releases only on
    verified `DELETED`; staging releases only after authoritative metadata
    publication or verified cleanup/reconciliation. Unknown never frees quota.
15. Delete claim requires exact current generation/head, internally recomputed
    zero active references, expired retention/grace, exact quota projection,
    valid database time, root/daemon/epoch/admission binding, and a typed
    fixed-owner sweep receipt/current-head pair.
16. `DELETE_CLAIMED` has one exact non-zero token and blocks new references
    and normal read claims. Exact token retry precedes stale/time evaluation.
17. Read claims use typed owner authority, object-scoped exact acquire/release
    commands, and a maximum 15-minute lease. Expiry becomes
    `EXPIRED_SUSPECT`, remains quota/delete blocking, and requires verified
    holder-death or handle-closure reconciliation before terminal release.
18. Delete claim requires zero active read claims in addition to zero
    references; the claim blocks new reference and normal read claims.
19. A verified no-effect failure may return to `AVAILABLE`; an ambiguous
    filesystem/transaction outcome enters `RECONCILIATION_REQUIRED` and can
    become `AVAILABLE` or `DELETED` only from verified metadata-plus-byte
    evidence. Unknown never implies success or safety.
20. Reintroduction after deletion uses a strictly higher non-wrapping
    generation; stale sweep plans cannot target it.
21. The object owns one receipt chain; task/project/store quota projections
    are separate fixed-owner aggregates updated atomically with it and require
    independent checkpoints for replay/restore.
22. Request, object, reference, head, terminal receipt, checkpoint, delete
    plan, claim, and result have separate canonical hash domains.
23. Raw artifact bytes never enter command receipts, metadata snapshots,
    errors, or `Debug`; the fake byte backend is separate from replay metadata.
24. The pure owner performs no I/O and exposes no real deletion operation.
25. Managed-foreman evidence uses a closed kind, exact task/project/attempt
    binding, separate byte and descriptor hash domains, and rejects common
    credential-bearing content. PostgreSQL may retain its verified bytes and
    descriptor only; an unverified row cannot become task evidence.
26. The pre-dispatch baseline is recorded after atomic attempt claim and before
    the first provider thread RPC. Retry may create a new attempt descriptor,
    but its baseline bytes/content digest must equal the first durable value.

## Allowed Dependencies

- Rust standard library.
- `lattice-contracts` 1.6 immutable shared values.
- `lattice-cjson` 1.0 canonical metadata-byte mechanics.
- Exact pinned SHA-256 and canonical time parsing/formatting mechanics.

## Forbidden Dependencies

- Policy, Task Domain, Task Ledger, Project Registry, Writer Lease, Approval
  Verifier, ports, PostgreSQL/store adapters, filesystem clients, Workspace
  Git, Scope Check, Orchestrator, Review Runtime, Guardian, provider adapters,
  Codebase Memory, CLI/app layers, product repositories, or concrete I/O
  clients.

## Failure, Compatibility, And Migration

Unknown, malformed, missing, over-limit, quota-exhausted, digest-mismatched,
stale, cross-project, wrong-generation, wrong-reference, fake/live-mixed,
reused, corrupt, expired, retained, unauthorized-owner, wrong-token,
outcome-unknown, or unsupported input fails closed with stable typed errors or
terminal denials. Denial never partially changes object, generation, bytes,
reference, or quota state.

A future PostgreSQL adapter must reuse the public planner/verifier, serialize
metadata/reference/current-head/quota/delete-claim state, enforce daemon
epoch/admission and database time in the same transaction, represent
unknown outcomes durably, and must not duplicate lifecycle or hash semantics.

A future filesystem adapter must stream to an owned staging file, enforce the
same incremental bounds/digest, flush data and directory metadata, atomically
publish under an internal path, verify reads, reject links/junction escapes,
and unlink only one exact claimed object. It cannot infer retention from a
directory listing.

Its future constitution must additionally enforce root physical identity and
product-root ancestor/descendant exclusion, case-fold collision checks,
Windows reparse/junction/symlink/hardlink/ADS/device/non-regular-file denial,
same-volume staging, exclusive temporary creation, no-clobber rename,
directory flush, handle/file-identity TOCTOU checks, bounded bundle
normalization, active-read/delete serialization, and orphan quarantine. These
are documented gates now; TASK-016 does not claim them machine-enforced.

## Acceptance Gates

| Gate | Evidence | Owner | Required for merge |
|---|---|---|---|
| Shared object/reference/receipt | fixed producer/version plus complete substitution matrix | Engineering | yes |
| Byte verification/bounds | empty, exact limit, over-limit, length/digest mismatch tests | Security review | yes |
| Aggregate quota | manifest/object/reference/task/project/staging exact-limit and atomic-counter tests | Security review | yes |
| Quota release and aggregation | unique/per-task/per-reference accounting, store/history limits, delete/unknown/orphan worst-case retention | Security review | yes |
| Project/generation isolation | cross-project, object, generation, and reintroduction matrix | Security review | yes |
| Provenance non-authority | provider/source substitution and receipt-owner tests | Architecture review | yes |
| Reference lifecycle authority | typed owner receipt/current head, action/scope/generation substitution, release/terminal reuse tests | Security review | yes |
| Idempotency | exact scoped key, complete sanitized request retention, separate hash domains, retry/changed/stale/zero-partial mutation | Security review | yes |
| Retention/delete claim | zero-reference, deadline/grace, typed sweep authority, claim token, stale-plan, retain/read block, known-failure, unknown/reconciliation tests | Security review | yes |
| Read claim lifecycle | typed acquire/release, exact retry, hard limits, expiry-suspect, holder reconciliation | Security review | yes |
| Provenance authority binding | Registry/effect/daemon/admission/capability/adapter/limit-snapshot mutation matrix | Security review | yes |
| Replay/rollback | raw corruption, denied-tail chain, and trusted-checkpoint matrix | Engineering | yes |
| Dependency/no-I/O/secrets | Cargo tree plus forbidden source and raw-byte leak scans | Architecture review | yes |
| Full verification | workspace format, lint, Rust and preserved Node tests | Engineering | yes |
| Managed foreman evidence | byte/descriptor separation, size boundary, secret-shaped rejection, tamper replay, and `Debug` redaction | Security review | yes |
| Managed worktree baseline | predispatch ordering, content-bound `worktree_ref`, exact retry, and control/index drift tests | Security review | yes |

## Change Policy

Mission, object/reference schema, project isolation, hash/size limits,
generation/reference lifecycle, idempotency, currentness, retention/sweep
ownership, public receipts, dependency direction, or failure behavior changes
require a versioned amendment, SPEC/ADR trace, security and architecture
review, and responsible-user authorization.

## Amendment History

| Version | Date | Decision reference | Summary | Approver |
|---|---|---|---|---|
| 1.0 | 2026-07-30 | SPEC-002 v12, ADR-014, TASK-016 | Pure project-scoped artifact owner, deterministic fake, provenance/reference receipt/head, replay, and sweep plan split | User MVP-3 execution directive |
| 1.1 | 2026-08-26 | SPEC-011, ADR-028 | Add bounded immutable managed-foreman evidence and strict rehydration without granting workflow authority | Delegated product owner |
