---
module_id: lattice-contracts
name: LATTICE Shared Contracts
version: 1.9
status: active
owner: LATTICE maintainers
last_reviewed: 2026-08-02
---

## Mission

Own the smallest versioned, immutable, I/O-free Rust value and receipt types
shared across LATTICE module boundaries without becoming the semantic owner of
their mutable domain state.

## Non-Goals

- Define task-domain, Registry, orchestration, policy, approval, or artifact
  transitions/rules.
- Perform serialization, hashing, persistence, process control, or provider I/O.
- Freeze canonical JSON or any live external protocol.

## Owned Data

- In-process request, task, attempt, project, snapshot, and SHA-256 reference
  values.
- Shared project class/lifecycle, physical local Git-ref identity, and
  immutable Project Registry authority receipt/full-head representations.
- Immutable Task Ledger stream-head, checked resource usage, and resource
  observation receipt/full-head representations.
- Immutable Writer Lease identity, signed-BIGINT-safe epoch/fence/revision,
  runtime-admission, and authority receipt/full-head representations.
- Complete neutral typed approval subjects, challenge/authority identity,
  positive approval revision, availability, and fixed-producer authority
  receipt/full-head representations.
- Project-scoped artifact object/generation, immutable reference/provenance,
  bounded byte-length/quota/read-claim, purpose/availability/delete-claim, and
  fixed-producer artifact receipt/full-head representations plus typed
  initial/reference/read-owner and sweep authority receipt/current-head
  representations.
- Component, boundary, runtime, invocation, and normalized evidence types.
- Neutral bounded gateway peer, request/action payload, reply/disposition,
  stable denial, and redacted Task Spec document representations.
- Neutral bounded Store transaction/daemon identifiers, closed repository
  owner, project/snapshot/aggregate scope, daemon authority head, physical
  compare-and-swap head, request commitments, terminal disposition/receipt,
  explicit fake/PostgreSQL durability classification, and immutable database-
  identity/schema-manifest persistence evidence.
- No durable or mutable project truth, credential, or provider-session data.
  Shared receipts carry immutable identifiers and digests only; Project
  Registry retains lifecycle and issuance ownership.

## Public Contracts

- Reject empty identifiers and malformed lowercase SHA-256 references.
- Validate canonical project IDs and only fully qualified local
  `refs/heads/*` physical Git-ref identities. Reject an explicit closed
  pseudo-ref denylist and ambiguous nested namespaces without rejecting valid
  uppercase branch names such as `WIP` or `RELEASE_2026`.
- Reject unknown contract versions before a port call.
- Preserve immutable invocation identity in normalized evidence.
- Distinguish component, authority boundary, and fake/live runtime identity.
- Expose only lane-specific evidence constructors with fixed
  component/boundary pairs.
- Construct task-agnostic Project Registry authority receipts only for the
  fixed `lattice-project-registry` producer ID and supported semantic producer
  version.
- Project every security-relevant receipt field into a full authority head:
  producer/version, runtime, project/snapshot, non-zero revision,
  lifecycle/class, primary ref, observation digest, and receipt digest.
- Construct Task-Ledger-owned resource observations only for fixed producer
  `lattice-task-ledger`, semantic version `2.0`, and an exact fake/live runtime.
- Bind the complete project/snapshot/task/revision/spec, stream/head,
  resource-projection revision/digest, effect claim, currency, checked
  current/requested counters, observation digest, and receipt digest.
- Project every resource receipt security field into a full observation head.
- Construct Writer-Lease-owned authority receipts only for fixed producer
  `lattice-writer-lease`, semantic version `1.0`, and an exact fake/live
  runtime.
- Validate positive signed-BIGINT-compatible daemon epoch, fencing token, and
  Writer Lease revision values in `1..=i64::MAX`.
- Bind complete project/snapshot/task/revision/spec/attempt, lease/holder/
  worktree/process-start, daemon instance/epoch/fence, status, admission,
  timestamp, observation, transition, and receipt fields.
- Project every Writer Lease authority-receipt security field into a full
  authority head.
- Expose one complete typed approval subject graph shared by Approval Verifier
  and Policy: binding, execution/cost, merge, preference, protected-change,
  release/guardian, and upgrade-delta values.
- Construct Approval-Verifier-owned authority receipts only for fixed producer
  `lattice-approval-verifier`, semantic version `1.0`, and exact fake/live
  runtime.
- Bind requester, approver, authority/trust lane, channel/session, nonce
  commitment, challenge, issue/expiry, complete typed subject, authenticator/
  key identity, proof/evidence/review-set, revision/status, and every digest.
- Project every approval authority-receipt security field into a complete
  authority head.
- Construct Artifact-Store-owned receipts only for fixed producer
  `lattice-artifact-store`, semantic version `1.0`, and an exact fake/live
  runtime.
- Bind complete project/snapshot/task/revision/spec/attempt/request,
  project-scoped object/digest/generation, byte length, media/schema,
  bundle bounds, producer/version/runtime/binary, adapter/version/binary,
  invocation/correlation/run/sequence/produced-at/payload, capability/input/config/
  evidence, Registry authority, effect claim, daemon instance/epoch/admission,
  capability-owner receipt/head, limit snapshot, purpose/retention/reference/
  revision/availability/delete-claim, manifest, observation, transition, and
  receipt fields.
- Project every artifact receipt security field into a complete artifact head.
- Represent initial publication/reference, retain/release/read and
  delete-claim authority only as typed fixed-owner receipt plus independently
  queried head pairs bound to exact owner record/revision/status, action,
  project/task/object/generation/reference/read, runtime,
  root/daemon/epoch/admission, and receipt digests.
- Represent only the six action-specific normal gateway requests, with action
  derived from the closed variant and no arbitrary SQL/shell/path/provider or
  protected-release escape hatch.
- Keep peer context outside request bytes and expose visibly fake construction
  only in TASK-017; representation never proves live OS authentication.
- Bind typed replies to command/correlation/action/request digest and keep stop
  request, terminal stop, denial, and unknown outcome distinct.
- Own neutral gateway representation constants and constructor-level
  identifier/cursor/page bounds; Gateway IPC separately owns wire layout,
  parser limits, canonical-NFC enforcement, hash subjects, and replay.
- Bound gateway-reused task, snapshot, and attempt identifiers to 256 bytes;
  reject zero authority, freshness, receipt, observation, routing, and terminal
  evidence digests rather than treating a sentinel as proof.
- Bound a project-status reply by both the protocol maximum and the originating
  request page size, and validate reply structure before canonical hashing.
- Represent one complete Store transaction without SQL/schema/table/path/
  arbitrary-record fields, and expose no caller Boolean that grants domain or
  durability authority.
- Bind Store receipts to fixed producer/version, exact request/scope/authority/
  before/after/commitment fields, disposition, transaction digest, receipt
  digest, runtime, durability, and any required persistence evidence. Contract
  v1 remains fake-only; v2 permits either Fake/NonDurableFake without
  persistence evidence or Live/DurablePostgres with non-zero database identity
  and manifest commitments plus a positive schema version.
- Keep Store physical heads nominally distinct from every domain-owner head;
  neither a physical head nor Store receipt proves domain legality/currentness.

## Invariants

1. Contract construction performs no I/O or hidden normalization.
2. Unknown versions and malformed values fail closed.
3. A public caller cannot construct a mismatched component/boundary pair.
4. A fake runtime marker cannot be confused with live or durable evidence.
5. Contracts do not depend on ports, adapters, orchestration, or policy.
6. Project receipt representation never grants Registry mutation, Policy
   authority, persistence, or freshness by itself.
7. Project IDs and local Git refs perform validation without hidden
   normalization; semantic canonicalization belongs to their owner module.
8. `ProjectAuthorityReceipt::head()` is a structural projection of that
   receipt, not proof of currentness. A consumer must compare it with an
   independently obtained current owner head.
9. A caller cannot substitute the Registry producer ID or semantic producer
   version, and changing any security-relevant receipt field changes full-head
   equality.
10. Resource receipt representation never grants Ledger mutation, persistence,
    freshness, Policy authority, or effect admission by itself.
11. `TaskLedgerResourceReceipt::head()` is a structural projection, not proof
    of an independent current owner lookup. Every security-relevant field is
    mirrored for exact equality.
12. Writer Lease receipt representation never grants lease mutation,
    persistence, process-death authenticity, runtime-admission authority, or
    Policy admission by itself.
13. `WriterLeaseAuthorityReceipt::head()` is a structural projection, not
    proof of an independent current owner lookup. Every security-relevant field
    is mirrored for exact equality.
14. Shared Writer Lease numeric values cannot represent zero, negative, or
    values above PostgreSQL signed `BIGINT`.
15. Approval subject representation cannot express a contradictory
    kind/subject pair; subject kind is derived from the closed typed variant.
16. Protected release cannot be paired with a responsible-user/normal trust
    lane.
17. Approval receipt representation grants no proof authenticity, currentness,
    nonce availability, claim authority, Policy admission, or independent
    review acceptance.
18. `ApprovalAuthorityReceipt::head()` is structural only. Every
    security-relevant field is mirrored for exact equality with an independently
    obtained owner head.
19. Artifact object identity cannot omit its project namespace or positive
    generation; equal digests from different projects are different values.
20. Artifact receipt representation grants no byte persistence, producer
    authenticity, content trust, reference currentness, deletion, Policy,
    memory, approval, or release authority by itself.
21. `ArtifactAuthorityReceipt::head()` is structural only. Every
    security-relevant field is mirrored for exact equality with an
    independently obtained owner head.
22. Shared artifact lengths, generations, and revisions cannot exceed
    PostgreSQL signed `BIGINT`; length may be zero while generation/revision
    remain positive.
23. Artifact authority representation cannot encode caller-owned reference
    counts, quota verdicts, retention Booleans, producer strings, or bare
    evidence digests as permission.
24. Artifact quota/read/delete representations distinguish worst-case occupied
    claim/reconciliation/orphan state from verified quota release; expiry does
    not itself release a read claim.
25. Gateway request representation cannot encode a caller-owned authenticated,
    admin, human, protected, SQL, shell, provider, path, daemon, process, or
    Guardian Boolean/string escape hatch.
26. A Task Spec document is bounded and redacted in `Debug`; representation
    does not validate Task Domain semantics or grant task creation authority.
27. Gateway-reused task, snapshot, and attempt identifiers are at most 256
    bytes; the exact 256-byte boundary remains valid and 257 bytes fails closed.
28. Gateway authority, freshness, receipt, observation, routing, and terminal
    evidence digests cannot use the all-zero SHA-256 sentinel.
29. A project-status reply cannot contain more tasks than the originating
    request page size or the protocol maximum, and invalid reply structure must
    be rejected before canonical hashing or replay insertion.
30. Store transaction identifiers and daemon identifiers are bounded canonical
    ASCII; Store security commitments reject the all-zero digest sentinel.
31. Store scope cannot omit or mix project, snapshot, owner, or aggregate
    identity, and repository owner is a closed enum rather than a caller string.
32. Store physical revisions fit non-negative signed PostgreSQL `BIGINT`, and
    increment responsibility remains with the Store implementation.
33. Store contract v1 cannot encode live runtime, durable commit, persistence
    evidence, domain authority, protected action, or migration execution.
34. Store contract v2 accepts only exact Fake/NonDurableFake/no-persistence or
    Live/DurablePostgres/complete-persistence combinations; mixed combinations
    fail construction.
35. A live durable physical receipt proves only that its complete opaque
    physical transaction row committed under the bound database/schema
    evidence. It never proves domain legality, domain currentness, provider
    effect delivery, Guardian activation, or protected release authority.

## Allowed Dependencies

- Rust standard library.

## Forbidden Dependencies

- `lattice-ports`, concrete adapters, database/network/process clients, model
  SDKs, serialization frameworks, credentials, and product repositories.

## Failure, Compatibility, And Migration

Invalid values return typed construction errors. Version 1.9 preserves v1
fake-only request/receipt compatibility and adds v2 live/durable PostgreSQL
receipt representation with complete persistence evidence. It still performs
no hashing, I/O, current-state query, durability verification, or domain
decision. Version 1.8 adds neutral
Store transaction, authority, physical-head, commitment, disposition, receipt,
and non-durable fake values. It does not canonicalize/hash a request, perform a
transaction, query current state, validate domain legality, execute SQL or a
migration, or prove PostgreSQL durability. Version 1.7 adds neutral
bounded gateway peer/request/reply representations and replaces the nominal
`Invocation + GatewayAction` service command with action-specific values. It
does not parse or hash frames, enforce wire NFC, authenticate a peer, route a
workflow, persist idempotency, validate Task Spec semantics, or grant
approval/stop authority.
Version 1.6 adds neutral
project-scoped artifact object/generation, immutable reference/provenance,
bounded length, purpose/availability, fixed-producer receipt/head, and typed
reference-owner/sweep authority receipt/head representations. It does not hash
bytes or manifests, authenticate producers, issue references, query current
state, decide trust/retention, store bytes, or delete anything. Version 1.5
approval behavior is unchanged.
Version-1 adapter invocation/evidence behavior is unchanged. Changing a public
field, meaning, identifier rule, or supported version requires a versioned
constitution amendment plus compatibility evidence.

## Acceptance Gates

| Gate | Evidence | Owner | Required for merge |
|---|---|---|---|
| Contract tests | `cargo test -p lattice-contracts` | Engineering | yes |
| Project identity/receipt substitution | focused constructor and equality matrix | Security review | yes |
| Ledger resource receipt substitution | fixed producer/runtime plus full receipt/head equality matrix | Security review | yes |
| Writer Lease receipt substitution | fixed producer/runtime, signed-BIGINT bounds, and full receipt/head equality matrix | Security review | yes |
| Approval subject and receipt substitution | complete typed subject plus fixed producer/runtime/trust-lane/full-head matrix | Security review | yes |
| Artifact object/reference/authority substitution | project/generation/bounds, fixed producer/runtime/full-head, owner action/scope, and sweep claim matrix | Security review | yes |
| Gateway boundary/substitution matrix | identifier and digest bounds, request/reply variant binding, page limit, role-before-replay, and fake replay capacity | Security review | yes |
| Store transaction/substitution matrix | identifier/scope/digest/revision/runtime/durability and complete receipt-field checks | Security review | yes |
| Dependency inspection | Cargo metadata shows no dependencies | Architecture review | yes |
| Full Rust verification | workspace format, lint, and tests | Engineering | yes |

## Change Policy

Mission, public types, validation rules, or dependency direction changes require
a versioned amendment, SPEC-002 trace, architecture review, and user approval.

## Amendment History

| Version | Date | Decision reference | Summary | Approver |
|---|---|---|---|---|
| 1.0 | 2026-07-29 | SPEC-002 v3, ADR-004 | Shared adapter-boundary contracts | User |
| 1.1 | 2026-07-29 | SPEC-002 v7, ADR-010, TASK-012 | Shared Project ID/class/lifecycle, physical Git-ref identity, and minimal Registry authority receipt/head | User MVP-3 execution directive |
| 1.2 | 2026-07-29 | SPEC-002 v8, ADR-010 review amendment, TASK-012 | Fixed Registry producer/version, full security-field head projection, and explicit pseudo-ref denial with valid uppercase branches preserved | User MVP-3 execution directive |
| 1.3 | 2026-07-29 | SPEC-002 v9, ADR-011, TASK-013 | Fixed Task Ledger producer/version plus neutral full stream head and resource observation receipt/head representation | User MVP-3 execution directive |
| 1.4 | 2026-07-29 | SPEC-002 v10, ADR-012, TASK-014 | Fixed Writer Lease producer/version plus neutral signed-BIGINT identity, runtime admission, and full authority receipt/head representation | User MVP-3 execution directive |
| 1.5 | 2026-07-29 | SPEC-002 v11, ADR-013, TASK-015 | Complete neutral typed approval subjects plus fixed Approval Verifier identity, revision, availability, and full receipt/head representation | User MVP-3 execution directive |
| 1.6 | 2026-07-30 | SPEC-002 v12, ADR-014, TASK-016 | Project-scoped artifact object/generation, immutable provenance reference, bounded length, availability, and full receipt/head representation | User MVP-3 execution directive |
| 1.7 | 2026-08-01 | SPEC-002 v13, ADR-015, TASK-017 | Bounded typed gateway peer/request/reply representation with protected and stop separation | User MVP-3 execution directive |
| 1.8 | 2026-08-01 | SPEC-002 v15, ADR-016, TASK-018 | Neutral typed Store transaction, daemon authority, physical head, commitments, terminal receipt, and explicit non-durable fake representation | User MVP-3 execution directive |
| 1.9 | 2026-08-02 | SPEC-002 v22, ADR-018, TASK-020 | Preserve Store v1 fake-only compatibility and add v2 live PostgreSQL durability plus database/schema persistence evidence | User MVP-3 execution directive |
