# ADR-010: Project Registry Receipts And Identity Ownership

- Status: accepted for TASK-012 under the approved V2 module direction and the
  user's directive to continue LATTICE DevOS through MVP-3
- Date: 2026-07-29
- Decision owner: user
- Related: SPEC-002 v8, ADR-005, ADR-008, ADR-009, TASK-012

## Context

Project Registry must become the sole semantic owner of registered project
identity, immutable project snapshots, lifecycle, drift, suspension, and
reconciliation. Policy already consumes a task-bound `ProjectAuthorityFact`,
but that fact currently carries caller-supplied booleans and Policy-local
`ProjectClass` and `GitRefIdentity` types. Letting Project Registry depend on
Policy would reverse the approved dependency direction; making Policy depend
on Project Registry would violate Policy Engine's no-owner-module dependency
rule.

The existing shared contracts contain `ProjectSnapshotId` but no canonical
project identifier, shared physical Git-ref identity, or immutable owner
receipt. Project Registry also needs deterministic command replay semantics
before PostgreSQL supplies physical persistence.

## Decision

### Shared value and receipt boundary

`lattice-contracts` 1.2 owns only the smallest immutable, I/O-free value shapes
needed across Registry, Policy, future Workspace Git, and Orchestrator:

- validated canonical `ProjectId`;
- `ProjectClass` and `ProjectLifecycle` wire values;
- a fully qualified local `refs/heads/*` `GitRefIdentity` whose physical
  storage digest is supplied by an authoritative inspector; an explicit
  pseudo-ref denylist rejects revision aliases without rejecting valid
  uppercase branch names;
- task-agnostic `ProjectAuthorityReceipt` and `ProjectAuthorityHead` values
  carrying fixed Registry producer/version, runtime identity,
  project/snapshot, Registry revision, lifecycle/class, primary ref,
  observation digest, and receipt digest. The head mirrors every
  security-relevant receipt field.

These types do not own a durable project record or legal lifecycle transition.
They validate representation only. Project Registry owns their semantic
production; Policy owns only the decision sufficiency rules applied to them.
`ProjectAuthorityReceipt::head()` is a structural projection, not currentness
proof.

### Registry ownership

Create pure Rust `lattice-project-registry` 1.1:

- it owns canonical root, root identity, repository identity, filesystem/file
  identity, primary-ref identity, lifecycle, revision, immutable snapshots,
  duplicate detection, drift classification, suspension, and reconciliation;
- it accepts an immutable `RepositoryObservation` in TASK-012 and performs no
  filesystem or Git inspection itself;
- its deterministic fake owner issues only `RuntimeKind::Fake` receipts;
- register, suspend, drift observation, and successful reconciliation rotate
  both Registry revision and immutable snapshot; an exact no-drift resolve
  reuses the current head;
- command ID, canonical-root text, and primary-ref text must already be NFC so
  semantic aliases cannot create distinct command/hash subjects through hidden
  normalization;
- accepted identities take precedence over pending observations. The first
  non-colliding pending observation reserves its identities for its owning
  project, and another project cannot front-run that reservation;
- ordinary duplicate registration or reconciliation returns `Denied` without
  mutation;
- when an `ACTIVE` project's authoritative observation collides with another
  project's accepted or pending identity, retaining its old `ACTIVE` authority
  is unsafe. Registry returns `Blocked`, rotates the observed project to a new
  `SUSPENDED` snapshot/head, clears its colliding pending observation, and
  leaves the other project's reservation unchanged;
- moved, replaced, suspended, stale, or cross-project evidence never yields an
  active current authority;
- project class is immutable after registration.

The real Windows/filesystem/Git inspection algorithm is deferred to the
Workspace Git/inspection ticket. In particular, the physical Git-ref digest
must remain stable across loose-ref and packed-ref representation changes; the
fake digest in TASK-012 proves comparison semantics only.

### Exact command receipts

Every fake Registry command, including an exact read-only observation, is
executed through a domain-separated command
subject with:

- command ID;
- canonical request digest;
- expected full authority head for observe, suspend, and reconcile; register
  has no prior head;
- before/after authority heads;
- typed terminal outcome, including distinct `Denied` and state-changing
  `Blocked` encodings;
- result digest.

Replaying the same command ID and same request returns the identical terminal
receipt, including an exact no-mutation observation receipt. Reusing a command
ID with a different request is rejected. PostgreSQL will persist and serialize
these semantics in a later ticket; the in-memory fake is not durable truth.

### Policy projection

Policy Engine 2.3 keeps its task-specific `SubjectBinding`, but replaces
caller-owned registered/active/drift booleans with:

- one validated `ProjectAuthorityReceipt`;
- one `ProjectAuthorityHead` supplied by an independent current Registry-owner
  lookup.

Policy requires exact equality of every full-head security field between the
receipt projection and current owner lookup, in addition to exact
project/snapshot equality with the Task Spec binding. It accepts only `ACTIVE`;
`SUSPENDED` and `RECONCILIATION_REQUIRED` deny. Fake Registry evidence is valid
only for a fake Task Spec runtime. Policy still has no direct Registry
dependency and never authenticates, persists, refreshes, or mutates a receipt.

Pure Policy cannot determine that a self-consistent historical receipt/head
pair has become stale if the caller merely re-projects the old receipt. The
independent lookup rule is documented-only in TASK-012; future
Orchestrator/PostgreSQL composition must authenticate and serialize the latest
Registry head before live authority is claimed.

The future Orchestrator is responsible for composing a Registry receipt with
the exact Task Spec. Registry never invents a task ID or Task Spec hash.

### Scope Check composition gate

TASK-012 does not implement Scope Check. A future exact Scope Check receipt
must be composed by Orchestrator and bind at least:

- project ID, project snapshot, Registry revision, Registry receipt digest, and
  Registry observation digest;
- Task Spec hash and revision;
- reviewed commit, target head, and diff digest;
- Scope Check rule-set hash, report digest, producer/version, and current
  observation revision.

Workspace Git owns merge-readiness analysis; Scope Check owns changed-path
classification. Neither may replace Project Registry as project-identity
owner.

## Dependency Direction

```text
lattice-project-registry
  -> lattice-contracts
  -> lattice-cjson

lattice-policy
  -> lattice-task-domain
  -> lattice-contracts

future lattice-orchestrator
  -> lattice-project-registry
  -> lattice-policy
```

There is no Policy/Registry cycle and neither module imports the other's
private state.

## Consequences

- Shared values have one representation without moving mutable project truth
  into `lattice-contracts`.
- Policy can distinguish a current exact owner receipt from a stale or
  substituted receipt when given an independent current owner head, without a
  meaningless standalone `fresh` Boolean.
- TASK-012 remains deterministic and I/O-free; it does not prove real Windows
  canonicalization, Git physical identity, PostgreSQL durability, restart
  behavior, or Scope Check.
- Review-driven security hardening requires Project Registry 1.1, Contracts
  1.2, and Policy 2.3.

## Verification

- Shared-contract construction/rejection tests for project IDs, explicit
  pseudo-ref denial, valid uppercase branches, fixed producer/version, full
  heads, and structural-projection semantics.
- Registry registration, accepted/pending reservation, front-run prevention,
  zero-mutation duplicate denial, defensive cross-project blocking, lifecycle,
  drift, reconciliation, snapshot immutability, NFC rejection, command replay,
  and substitution matrices.
- Policy independent-current-head and full-field
  receipt/head/project/snapshot/runtime substitution tests.
- Dependency inspection and a forbidden-I/O source scan.
- Full Rust and preserved Node verification plus independent review.
