# ADR-014: Project-Scoped Artifact Identity, Provenance, And Sweep

- Status: accepted for TASK-016 under the user's directive to continue the
  approved LATTICE plan through MVP-3
- Date: 2026-07-30
- Decision owner: user
- Related: SPEC-002 v12, ADR-004 through ADR-009, ADR-010 through ADR-013,
  TASK-016

## Context

Graphify graphs, Hermes candidates, Codex evidence, independent-review
bundles, Codebase Memory sources, and upgrade candidates all need immutable
bytes and provenance. The current V2 proposal says Artifact Store owns
content-addressed bytes while PostgreSQL stores authoritative references, but
it does not yet freeze:

- whether equal bytes from different projects share an identity or path;
- which project/task/attempt and producer fields are immutable;
- what an Artifact Store receipt proves;
- how duplicates, retries, reference races, rollback, and reintroduction work;
- which authority may declare an object unreferenced and safe to delete;
- where pure semantics end and PostgreSQL/filesystem mechanics begin.

Adding provider or database adapters first would let each adapter invent an
artifact identity, provenance envelope, or cleanup rule. That would create
alternate authorities and make cross-project leakage or unsafe deletion
likely.

## Decision

Create pure Rust `lattice-artifact-store` 1.0. It is the sole semantic owner
of:

- project-scoped content-object identity and positive generation;
- immutable reference/provenance manifests and their canonical hash domains;
- declared/observed byte length and raw SHA-256 verification;
- fixed object/reference bounds;
- per-task and per-project object/reference/byte/staging quota semantics;
- exact command idempotency and terminal receipt chains;
- reference-set/current-head, replay, trusted checkpoint, and rollback rules;
- retention/grace evaluation and exact sweep planning.

TASK-016 supplies a deterministic visibly non-durable in-memory fake. It
performs no filesystem, database, process, network, Git, provider, credential,
publication, deployment, or product-repository I/O.

## Project-Scoped Object Identity

One logical byte object is identified by:

```text
(project_id, algorithm = sha256, content_digest)
```

Equal bytes may deduplicate only inside the same project in 1.0. They do not
share an observable key, path, reference set, or lifecycle across projects.
This intentionally trades global deduplication for simple isolation and no
cross-project existence oracle.

Every present physical instance also has a positive signed-BIGINT-compatible
generation. After an exactly authorized sweep, later reintroduction of the
same project/digest allocates a higher generation. A stale sweep or reference
bound to an older generation cannot target new bytes.

User filenames, media types, schema IDs, producer IDs, task IDs, and other
metadata never determine a storage path. A future filesystem adapter derives
an internal path from an owned-root identity, project namespace, algorithm,
digest, and generation only.

## Immutable Reference And Provenance

Bytes and uses are separate:

- an object owns the project-scoped digest, length, generation, availability,
  and active-reference-set digest;
- each use owns one immutable reference ID and a complete manifest.

The manifest binds:

- project, project snapshot, task, task revision, Task Spec digest, attempt,
  request, and reference ID;
- object content digest, byte length, media type, payload schema ID/version,
  and generation;
- source producer ID/version, runtime kind, invocation, capability, input-set,
  configuration, evidence, producer binary, adapter identity/version/binary,
  correlation, run, sequence, canonical produced-at time, and payload digests;
- Registry snapshot authority receipt/head digests, exact effect-claim ID/
  digest, daemon instance/epoch, runtime admission, capability-owner receipt/
  current-head digests, and a hash-bound limit snapshot;
- reference purpose, typed reference-owner authority, and retention deadline;
- manifest and receipt digests.

Artifact Store validates shape, bounds, exact binding, and hashes. It does not
decide whether content is factually correct, extracted versus inferred,
review-approved, safe memory, approved code, or releasable.

The pure owner verifies that those fields are complete and consistently bound;
it does not authenticate them. A future live Orchestrator/PostgreSQL
transaction must compare Registry, effect-claim, daemon/admission, capability,
and reference-owner receipts with independently queried current heads in the
same transaction that reserves staging or publishes a reference. Stale daemon,
epoch, admission, snapshot, capability, or effect-claim evidence fails closed.

Graphify, Hermes, Codex, Review Runtime, Guardian, a model, or a product
repository may appear only as source provenance. They cannot issue
`lattice-artifact-store` receipts or mutate an object/reference aggregate
directly.

Initial publication plus its first reference, later retain, and release never
accept a caller Boolean, count, bare producer string, or opaque evidence digest
as authority. All three require a typed owner-authority receipt plus an
independently obtained complete current head. The pair binds one closed owner
kind, fixed semantic producer/version, fake/live runtime, owner
record/revision/status, exact project/task/object/generation/reference,
authorized `PUBLISH_INITIAL_REFERENCE`, `ADD_REFERENCE`, or
`RELEASE_REFERENCE` action, and receipt digest.

The initial fake accepts only visibly fake owner-authority pairs issued by its
test composition. A later live path remains unavailable until Task Ledger,
Codebase Memory, Review Runtime, or Guardian has a versioned owner contract and
the Orchestrator/PostgreSQL transaction authenticates its receipt/current head.

## What A Receipt Proves

A fixed `lattice-artifact-store`/`1.0` receipt proves only that the semantic
owner accepted the exact object/reference transition represented by the
receipt. In the fake runtime, it proves deterministic binding only.

It grants none of:

- producer authenticity beyond separately supplied evidence;
- factual truth, review acceptance, or memory promotion;
- task, policy, writer-lease, approval, merge, activation, or release
  authority;
- filesystem persistence, PostgreSQL durability, or live currentness.

`receipt.head()` is a structural projection. A consumer requiring live bytes
must compare the complete receipt with an independently queried available
Artifact Store current head and then perform a digest-verified read.

The object head mirrors object key, generation, revision, availability,
length/digest, active reference count/set digest, monotonic sweep-not-before,
active read-claim count/set digest, delete claim/token state, task/project/
store/staging quota projection digests, command high-water/tail, transition,
and receipt digest. A reference head mirrors its complete manifest, owner
authority, active/released status, revision, transition, and receipt digest.

## Bounds And Byte Verification

The 1.0 hard limits are:

| Resource | Hard maximum |
|---|---:|
| one object | 1 GiB (`1_073_741_824` bytes) |
| one canonical reference manifest | 64 KiB |
| active references on one object | 65,536 |
| active read claims on one object | 4,096 |
| objects attributed to one task | 100,000 |
| references attributed to one task | 1,000,000 |
| active read claims attributed to one task | 65,536 |
| active referenced bytes attributed to one task | 64 GiB |
| concurrently staged bytes attributed to one task | 4 GiB |
| concurrent staging streams attributed to one task | 8 |
| command records attributed to one object | 1,000,000 |
| command records attributed to one task | 5,000,000 |
| canonical command-history bytes attributed to one task | 1 GiB |
| available objects in one project | 1,000,000 |
| active references in one project | 10,000,000 |
| active read claims in one project | 1,000,000 |
| command records in one project | 100,000,000 |
| canonical command-history bytes in one project | 64 GiB |
| available unique object bytes in one project | 1 TiB |
| available objects in one store | 10,000,000 |
| active references in one store | 50,000,000 |
| active read claims in one store | 5,000,000 |
| available unique object bytes in one store | 8 TiB |
| concurrent staging bytes/streams in one store | 16 GiB / 64 |
| command records/canonical history bytes in one store | 500,000,000 / 256 GiB |
| bundle entries | 100,000 |
| bundle path depth | 64 |
| bounded identifier/media/schema/producer field | 256 UTF-8 bytes |

Composition may configure lower per-store, project, or task limits but never
higher limits. Length/counters are non-negative signed-BIGINT-compatible
values. Empty artifacts are legal and have the standard SHA-256 digest of
empty bytes.

Every command binds an immutable limit-snapshot digest. The owner loads that
snapshot independently; callers cannot replace it with looser numbers. Bundle
descriptors bind entry count, maximum depth, and total declared bytes; total
bytes also consume task/project quota. A bundle filename remains metadata and
never becomes a storage path.

A publish operation supplies an expected digest and declared length. The fake
hashes the supplied bytes and rejects a length/digest mismatch before
publishing an object or reference. A future streaming adapter must enforce the
same bound incrementally and may not read an unbounded artifact into memory.
It must reserve staging bytes before accepting a stream and release or
reconcile the reservation after every terminal outcome.

Metadata identifiers and media/schema fields are separately bounded and
cannot contain NUL, path separators where an identifier is expected, or be
used as a filesystem path.

Artifact Store owns checked quota deltas and projections. Publication,
reference mutation, sweep claim, and reintroduction atomically validate and
update the affected object, task, project, and staging counters. A caller
cannot supply a trusted current count or `within_quota` Boolean. TASK-016's
fake retains deterministic projections; PostgreSQL later persists them in the
same serializable transaction as metadata/reference changes.

Quota accounting is exact:

- project/store available-object and unique-byte counters count one
  `(project_id, digest, generation)` once while it is `AVAILABLE`,
  `DELETE_CLAIMED`, or `RECONCILIATION_REQUIRED`;
- task object/active-byte counters count the object once per task that has at
  least one active reference, regardless of additional same-task references;
- reference and read-claim counters count each active identity;
- every staging reservation counts its declared bytes and stream even when the
  same digest is staged concurrently;
- complete canonical request-source bytes and command rows count against
  object, task, project, and store history quotas, including denied commands.

An object/project/store unique-byte or object quota is released only after a
verified `DELETED` transition. `DELETE_CLAIMED` and
`RECONCILIATION_REQUIRED` retain worst-case object/byte accounting. A staging
reservation is released only after authoritative metadata publication or
verified cleanup/reconciliation of the exact staged/sealed bytes. A sealed
orphan remains worst-case staging/project/store usage until verified
reconciliation; crash, timeout, or unknown commit never frees quota.

## State, Idempotency, And Replay

The minimum object state is:

```text
ABSENT -> AVAILABLE(generation n) -> DELETE_CLAIMED
DELETE_CLAIMED -> DELETED
DELETE_CLAIMED -> AVAILABLE (verified no-effect failure)
DELETE_CLAIMED -> RECONCILIATION_REQUIRED (unknown outcome)
RECONCILIATION_REQUIRED -> DELETED | AVAILABLE (verified reconciliation)
DELETED -> AVAILABLE(generation n + 1)
```

Adding or releasing a reference advances the object revision and changes the
complete current head. References are immutable; a released reference cannot
be reused or rebound. Exact object bytes plus a new valid reference may reuse
the current generation.

Read claims are distinct immutable identities with typed owner authority and
the same object/generation/current-head binding. Acquire and release use the
same object-scoped idempotency/receipt chain. A claim has a canonical maximum
15-minute lease, but reaching expiry only moves it to `EXPIRED_SUSPECT`;
expiry does not reduce read-claim quota or permit deletion. Verified holder
death or handle closure is required to reconcile it to terminal `RELEASED`.

`DELETE_CLAIMED` binds a unique non-zero claim token, exact object head,
generation, database observation time, daemon instance/epoch/admission, root
identity, and expected internal object key. It blocks new references and
normal read claims. The same claim token is idempotent; a different token
cannot replace it.

A verified definite failure before any unlink may return the object to
`AVAILABLE` with a higher revision and immutable evidence. Timeout, crash,
unknown commit, ambiguous path result, or missing completion evidence enters
`RECONCILIATION_REQUIRED`; it never guesses `AVAILABLE` or `DELETED`.
Reconciliation verifies both authoritative metadata and the exact owned
filesystem object/digest before choosing one terminal result.

Exact command retry is checked before stale-head or time evaluation. The same
command ID and same canonical request returns the identical terminal applied
or denied receipt after later state advancement. Changed content under a used
command ID rejects permanently. A denied command performs no partial object,
byte, generation, or reference mutation but remains in the terminal receipt
chain.

The idempotency storage key is exactly:

```text
(project_id, algorithm, content_digest, command_id)
```

Command IDs may repeat only for another project/object key. Every durable
command record retains the complete sanitized canonical request source,
request digest, storage key, and terminal receipt; a denied command cannot be
reconstructed from an object transition alone. Raw artifact bytes are excluded
from the record and represented by their declared/observed length and digest.

The object aggregate is keyed by `(project_id, algorithm, content_digest)` and
owns its reference/read/command receipt chain. Task, project, and store quota
projections are separate fixed-owner aggregates with positive revisions,
predecessor-bound heads, command-history counters, and receipt tails. An
object transition and every affected quota projection update atomically.
Replay or restore must compare independently retained checkpoints for the
object and all affected quota aggregates; a self-consistent object stream
cannot prove global capacity currentness.

Request, object-record, reference, object-head, terminal receipt, checkpoint,
delete-plan, durable delete-claim, and delete-result use separate
`lattice-cjson-1` hash domains.

Raw replay rejects unknown versions/kinds, malformed fields, changed order,
duplication, truncation, orphan references, cross-project/object/generation
substitution, digest mismatch, fake/live mixing, reference-set disagreement,
and command high-water/tail disagreement.

Context-free replay proves internal consistency only. Rollback-sensitive
restore requires an independently retained checkpoint binding object identity,
generation, revision, availability, any delete claim/reconciliation identity,
quota projections, reference-set digest, command high-water/tail, and full
snapshot digest.

## Retention And Sweep

Publication creates at least one active reference. A reference may be released
only by an exact command carrying the matching typed reference-owner authority
receipt plus independently obtained current owner head. Both the owner pair
and Artifact Store reference/current-object heads bind the exact project,
task, object, generation, reference, and `RELEASE` action. Release is terminal
and does not erase its receipt or provenance.
Release never shortens the retention deadline already committed by that
reference. The object's sweep-not-before projection is monotonic within one
generation.

Sweep planning fails closed unless all of the following are true:

1. the object is available at the exact expected generation and current head;
2. the active-reference set and active-read-claim set are both empty;
3. every retention deadline and the configured grace interval have expired at
   an explicit canonical observation time;
4. internally recomputed task/project quota and reference projections agree;
5. a fixed-owner Artifact Store sweep-authority receipt and independently
   queried current head bind the same project/object/generation, zero-reference
   set digest, retention/grace observation, root identity, and `CLAIM_DELETE`;
6. active daemon instance/epoch/admission evidence is exact and current.

The pure crate returns an immutable claim plan; it has no public filesystem
unlink operation. A future PostgreSQL transaction must recheck the exact head,
reference rows, generation, quota projection, daemon epoch/admission, root
identity, and database time while durably recording `DELETE_CLAIMED` and its
unique token. Only then may a future filesystem adapter verify exact owned
root, path components, no link/junction escape, digest/generation, and claim
token before removing one object.

Success, verified no-effect failure, or unknown outcome is recorded through
the typed state machine above. Claim/reconciliation commands use exact-token
retry before stale/time evaluation. Arbitrary recursive deletion is forbidden.

TASK-016's fake may simulate plan/application to prove lifecycle semantics,
but that is not evidence that a real file was durably published or safely
deleted.

## Future Live Publication Saga

The live composition order is fixed:

1. PostgreSQL atomically verifies Registry/effect/daemon/admission/capability/
   limit authority and records one bounded staging reservation plus intent.
2. The filesystem adapter streams to an exclusive owned staging object,
   verifies size/digest, durably flushes, and seals through atomic no-clobber
   publication.
3. PostgreSQL rechecks the same claims and atomically records Artifact Store
   object/reference/current-head/quota metadata, then releases the staging
   reservation.
4. A crash after byte seal but before metadata publication leaves a
   non-authoritative quarantined orphan. It cannot be discovered and promoted
   from a directory scan.
5. A metadata reference may never point to missing, unsealed, unverified, or
   wrong-generation bytes. Unknown saga results enter reconciliation.

Graphify/Hermes/provider adapters receive only a project/task/attempt/effect-
scoped staging capability. They cannot retain, publish, release, claim delete,
access PostgreSQL, or call the concrete filesystem adapter. The composition
root coordinates all three boundaries; adapters never call one another.

Normal internal Artifact Store publication is not
`DeploymentIntent::PrepareArtifact`. That Policy enum denotes a deployment
preparation surface and remains denied by the generic gate. A future internal
artifact port/action must use the exact Artifact Store authority described
here and cannot reuse or weaken deployment policy vocabulary.

## Future Filesystem Gate

The later filesystem-adapter constitution/ticket may strengthen but must not
weaken these gates:

- the artifact root has an owner marker plus physical file identity and is
  neither an ancestor nor descendant of any registered product root;
- canonical case-folded comparisons reject root/key collisions;
- Windows reparse points, junctions, symlinks, hardlinks, alternate data
  streams, device paths, and non-regular files fail closed;
- staging and final object are on the same verified volume;
- temporary files use exclusive create and random owner-controlled names;
- streaming bounds/digest are enforced before data/directory flush;
- publish is atomic no-clobber rename; a concurrent loser verifies the winner
  before reuse;
- directory metadata is durably flushed where the platform supports it;
- handle/file-identity rechecks close path-based TOCTOU windows;
- multi-file provider output is normalized into individually addressed objects
  plus a bounded immutable bundle manifest;
- active read claims and delete claims serialize through PostgreSQL; unlink
  targets exactly one claimed regular file;
- crash after rename and before metadata publication yields a quarantined
  orphan that directory scanning can never promote to authority.

## Ownership And Dependency Direction

```text
lattice-contracts 1.6
  immutable artifact representation only

lattice-artifact-store 1.0
  -> lattice-contracts
  -> lattice-cjson
  -> exact SHA-256 and canonical time mechanics

future postgres-store adapter
  serializes Artifact Store metadata/reference semantics

future artifact-filesystem adapter
  performs owned-root byte staging/read/sweep mechanics

future Graphify/Hermes/Codex/review/memory adapters
  -> injected artifact port only
```

PostgreSQL remains the durable One Truth and may not invent artifact
transitions or hash subjects. The filesystem blob root is not truth.
Graphify/Hermes/providers never receive database credentials or a concrete
Artifact Store dependency; the composition root supplies a narrow port.

## Consequences

- Cross-project byte equality cannot leak through a shared object identity.
- Provider output becomes provenance, not authority.
- Reference races and old sweep plans fail against generation/current-head
  changes.
- Caller-supplied counts, Booleans, and opaque digests cannot authorize
  retention release or deletion.
- Durable delete claims and reconciliation prevent an unknown filesystem
  outcome from being guessed safe.
- Aggregate quotas bound many-small-object, reference, metadata, and staging
  exhaustion, not only one large object.
- A deterministic fake can stabilize contracts before live storage exists.
- Real atomic write, crash durability, link containment, PostgreSQL
  transactions, restart, and unknown-outcome recovery remain explicit gates.

## Rejected Alternatives

- Global cross-project deduplication in 1.0.
- Use a filename, provider path, or database row ID as artifact identity.
- Let providers issue Artifact Store receipts or decide retention.
- Treat `receipt.head()` or a caller `referenced = false` Boolean as
  currentness.
- Accept a caller-supplied reference count, `within_quota` Boolean, or non-zero
  evidence digest as release/sweep authority.
- Transition directly from a pure sweep plan to `DELETED` without a durable
  claim token and unknown-outcome state.
- Let the filesystem directory listing become authoritative reference truth.
- Delete recursively from a caller-supplied path.
- Put PostgreSQL or filesystem calls inside the pure semantic crate.
- Treat artifact availability as factual trust, review approval, or release
  authorization.
