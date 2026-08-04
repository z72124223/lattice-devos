# ADR-013: Approval Subject, Proof, Currentness, And Nonce Claim

- Status: accepted for TASK-015 under the user's 2026-07-29 directive to
  continue the approved LATTICE plan through MVP-3
- Date: 2026-07-29
- Decision owner: user
- Related: SPEC-002 v11, ADR-002, ADR-005, ADR-007 through ADR-009,
  TASK-015

## Context

Policy 2.5 receives a public caller-constructible `ApprovalFact`. The caller
supplies the subject, authority, origin, actor/channel/session, timestamps,
nonce, and five security verdict Booleans: subject verified, identity
verified, fresh, nonce available, and self-approved. The supplied subject
digest is not recomputed, timestamps are only checked for non-empty text, and
R3 review sufficiency is also represented by two caller Booleans.

These values are useful V1 characterization but are not owner authority. Adding
PostgreSQL, OpenClaw approval IPC, or Guardian activation on top would make a
caller claim security properties that only an Approval Verifier, Review
Runtime, database transaction, or protected Guardian can establish.

## Decision

Create pure Rust `lattice-approval-verifier` 1.0. It is the sole semantic owner
of:

- complete typed approval-subject canonicalization and hash domains;
- challenge, nonce binding, fake proof verification, authority/trust-lane
  classification, expiry, availability, and normal claim preconditions;
- exact command idempotency, terminal receipts, aggregate replay, and trusted
  checkpoint comparison;
- fixed-producer authority receipts and independent current-head lookup.

The deterministic fake is non-durable semantic/composition evidence. It reads
no clock, randomness, environment, credential, key store, filesystem,
database, process, or network source.

## Shared Contract Boundary

`lattice-contracts` 1.5 carries the complete neutral typed subject graph used
by both Policy and Approval Verifier:

- task/project binding;
- execution and external-cost scope;
- merge target/commit/head/diff scope;
- memory preference candidate scope;
- protected-change class and operation scope;
- guarded release, guardian runtime, slots/epochs, manifest/source/binary/
  migration/evidence hashes, and capability delta.

Contracts also carries fixed `lattice-approval-verifier`/`1.0`
producer/version, runtime, identity, positive revision, availability, authority
receipt, and full authority-head representations. It does not canonicalize,
hash, verify, issue, query current state, read a clock, or claim a nonce.

An opaque caller-supplied subject digest is insufficient: Policy must compare
the complete typed subject in the owner receipt with the complete expected
decision subject. A receipt's own `head()` is structural projection only.

## Challenge, Proof, And Secrets

A challenge binds:

- approval and challenge IDs;
- complete project/snapshot/task/revision/spec subject;
- requester/proposer and approving actor identities;
- authority/trust lane, channel, session, and runtime;
- one nonce commitment;
- canonical issue and exclusive expiry times;
- subject, challenge, authenticator/key identity, proof, and evidence digests.

The only verified authority/trust pairs are responsible user plus
OS-authenticated user, and protected guardian plus Guardian trust root.
`ProtectedRelease` additionally requires the Guardian trust-root and runtime
identity inside its typed subject to match the protected proof lane. A normal
OpenClaw token, model, candidate, or active daemon cannot become protected
authority.

Raw nonce, OS token, private/MAC key, and raw authentication assertion never
enter the aggregate, receipt, snapshot, error, or `Debug`. Persisted forms
retain only commitments and digests. Fake proofs are domain-separated hashes
marked `RuntimeKind::Fake`; they make no cryptographic-security claim.

## State, Idempotency, And Replay

The minimum state is `CHALLENGED -> VERIFIED_AVAILABLE -> CLAIMED_NORMAL`, with
typed revocation and time-based unavailability. A protected release stops at
`VERIFIED_PROTECTED_PENDING_CLAIM`; TASK-015 exposes no general protected
consume command.

`REVOKE` is a terminal exact-head transition from
`VERIFIED_AVAILABLE` or `VERIFIED_PROTECTED_PENDING_CLAIM` only. The revoker
must be the original approving actor, the explicit canonical observation must
remain inside `issued_at <= observed_at < expires_at`, and the request must
carry a non-zero evidence digest. The transition advances the verifier
revision and emits an immutable typed revocation binding approval, revoker,
observation, prior authority receipt, and evidence. Challenged, claimed,
already revoked, stale, expired, or wrong-revoker requests deny without
partial authority mutation.

The deterministic fake proves only this binding and replay behavior. A live
normal-lane revoke must authenticate the original approver through the OS
trust adapter; a live protected-lane revoke must authenticate the original
Guardian approver through the Guardian trust-root adapter. Approval Verifier
1.0 defines no administrator, requester, gateway, model, daemon, or emergency
override. A different revocation authority requires a versioned ADR and
contract amendment.

One nonce commitment permanently binds one challenge/approval/subject/trust
lane, including after denial or expiry. Exact command retry precedes stale-head
and time evaluation. Same command and same request returns the identical
terminal receipt. Changed content under one command ID rejects permanently.
Denied commands do not mutate approval authority but remain in the terminal
receipt chain.

Every terminal command receipt binds its predecessor, request, before/after
head, outcome, and digest. The aggregate commits command high-water and tail.
Public raw replay rejects unknown versions/kinds, malformed fields, reordering,
truncation, duplication, orphan records, hash substitution, nonce rebinding,
fake/live mixing, and claimed-state disagreement.

Context-free replay proves only internal consistency. Rollback-sensitive
restore requires an independently retained checkpoint binding approval ID,
command high-water/tail, nonce-binding state, and full snapshot digest.

## Time And Currentness

`issued_at < expires_at`. Proof verification and normal claim are valid only
when `issued_at <= observed_at < expires_at`; equality with `expires_at` is
expired. All times are explicit canonical injected observations.

`current_head_at(approval_id, observed_at)` returns the complete head only when
the approval is verified, unclaimed, unrevoked, and time-valid. Query time is
not added to head equality. Policy reads no clock and accepts no caller
freshness Boolean.

A live effect must re-check the nonce row, status, authority, subject, database
time, daemon epoch/admission, and applicable resource/writer state while
holding the same PostgreSQL transaction lock. This closes the gap between a
prior current-head lookup and effect claim.

## Nonce Consumption Ownership

Approval Verifier defines pure claim-precondition semantics; it is not an
independent side-effecting consume service.

- A normal approval is claimed by the future transaction that performs or
  claims the exact approved task transition/effect.
- A protected release nonce is claimed only by the Guardian-only
  `claim_activation` transaction, atomically with receipt/subject validation,
  `ACTIVATION_CLAIMED`, and admission changing to `DRAINING`.

PostgreSQL owns serialization and physical durability but may not invent
approval transitions. Guardian owns protected activation order and authority
but may not rewrite Approval Verifier semantics.

## Policy 2.6 Boundary

Policy 2.6 removes the caller-owned approval verdict Booleans and consumes:

- an owner-produced authority receipt containing the complete typed subject;
- an optional complete current head obtained independently from the Approval
  Verifier owner.

Policy compares the complete binding, subject, authority/trust lane, runtime,
identity, nonce commitment, issue/expiry, revision, status, and all digests.
Missing, historical, expired, claimed, revoked, substituted, fake/live
mismatched, self-approved, or wrong-lane authority denies.

Policy keeps no production dependency on Approval Verifier or cjson. A
test-only dependency may prove composition with the deterministic fake.

`ReviewChecks { security, architecture }` is also removed as caller authority.
Approval receipts may bind a review-set digest but cannot prove that an
independent review passed. Until Review Runtime issues its own fixed-producer
receipt/current head, every R3 or `require_independent_checks` allow path fails
closed with an explicit review-authority-unavailable reason.

## Dependency Direction

```text
lattice-approval-verifier
  -> lattice-contracts
  -> lattice-cjson
  -> exact time parsing/formatting only

lattice-policy
  -> lattice-task-domain
  -> lattice-contracts

policy tests
  -> lattice-approval-verifier (dev-only)
```

Future Orchestrator composes Policy, the pure approval planner, and a
PostgreSQL transaction. The separate Guardian composition root has only the
protected claim path.

## Consequences

- Caller Booleans and unused subject digests can no longer manufacture
  approval authority.
- Normal and protected trust lanes become closed owner-validated state.
- Time, nonce availability, retry, and historical currentness become
  deterministic and testable without hidden I/O.
- R3 temporarily denies until a bounded Review Runtime owner ticket exists.
- Live authentication, cryptography, database clock/durability/restart,
  atomic nonce claim, OpenClaw IPC, and activation remain explicit future
  gates.

## Rejected Alternatives

- Keep caller security Booleans or merely rename them.
- Trust an opaque caller-supplied subject digest.
- Treat `receipt.head()` as independent currentness.
- Let Policy read a clock, verify a signature, or mutate a nonce.
- Let Approval Verifier independently consume a protected release nonce.
- Let PostgreSQL or Guardian reimplement subject, retry, or lifecycle meaning.
- Fold Review Runtime authority into Approval Verifier.
