---
module_id: approval-verifier
name: Approval Verifier
version: 1.0
status: active
owner: LATTICE maintainers
last_reviewed: 2026-07-29
---

## Mission

Own pure Rust, replayable approval-subject, challenge, proof, nonce-lifecycle,
availability, exact-command, and current-head semantics; issue fixed-owner
receipts through a deterministic non-durable fake; and provide the one
semantic core later reused by PostgreSQL and the protected Guardian claim.

## Non-Goals

- Decide Policy risk, approval floors, roles, actions, or whether a subject
  should be created.
- Decide whether independent correctness, security, or architecture review
  passed.
- Read an OS identity, trust root, key, clock, random source, environment, or
  credential.
- Persist or execute a task transition, product effect, merge, memory
  promotion, deployment, or activation.
- Expose a general command that consumes a protected-release nonce.
- Perform filesystem, Git, database, process, network, provider, credential,
  payment, publication, or deployment I/O.
- Claim a fake proof or receipt is live, cryptographically secure, durable, or
  OS-authenticated.

## Owned Data

- Complete approval subject field-selection and canonical hash semantics.
- Challenge, nonce commitment/binding, proof, authority/trust-lane, issue/
  expiry, availability, typed revocation, normal claim-precondition, and
  rejection meaning.
- Exact command receipts, predecessor chain, command high-water/tail,
  aggregate replay, and rollback checkpoint semantics.
- Fixed-owner receipt issuance and independent current-head projection.

Contracts owns shared immutable representation only. PostgreSQL owns physical
persistence/serialization only. Guardian owns protected activation authority
and order only.

## Public Contracts

- Accept a complete constructed typed subject; never choose or broaden it.
- Issue one challenge from explicit safe nonce commitment, time, requester,
  approver, authority/trust, channel/session, and evidence inputs.
- Verify one exact fake proof and return an immutable authority receipt.
- Query an independent complete head at an explicit observation time.
- Revoke one exact verified available or protected-pending authority only from
  its exact current head, inside its validity interval, by its original
  approving actor with a non-zero evidence digest; return an immutable typed
  terminal revocation and make later current-head lookup unavailable.
- Plan/apply a normal claim precondition; expose protected release only as
  pending material for future Guardian `claim_activation`.
- Return identical terminal receipts for exact retries and reject changed
  command content permanently.
- Export and verify a strict untrusted raw aggregate snapshot.
- Export and compare a validated trusted checkpoint for rollback-sensitive
  restore.

## Invariants

1. Binding, typed subject, requester, approver, authority/trust lane,
   channel/session, nonce commitment, issue/expiry, runtime, or any evidence
   digest substitution changes the challenge and receipt subject.
2. A nonce commitment is globally bound once and is never released for another
   approval, subject, or lane after denial, expiry, claim, or revocation.
3. `issued_at < expires_at`; valid observation is
   `issued_at <= observed_at < expires_at`.
4. Only responsible-user/OS-authenticated and
   protected-guardian/Guardian-trust-root pairs verify.
5. A protected-release subject verifies only on the protected Guardian lane
   whose trust-root/runtime identity exactly matches the subject.
6. A normal gateway, model, candidate, or active daemon cannot be transformed
   into protected authority.
7. Approver identity must differ from requester/proposer and any explicitly
   bound candidate identity; no caller self-approval Boolean exists.
8. Exact command retry precedes stale-head/time checks. Changed command content
   rejects permanently.
9. Applied and denied terminal receipts form one predecessor chain. Aggregate
   high-water/tail and a trusted checkpoint detect denial-tail loss and
   coherent-prefix rollback.
10. `receipt.head()` is structural only. Historical, claimed, revoked, or
    expired approval has no available independent current head.
11. Revocation is an exact-head, time-current terminal transition available
    only from verified normal authority or protected authority pending claim.
    The revoker is exactly the original approver and supplies a non-zero
    evidence digest. It advances revision, emits a typed immutable revocation,
    removes current-head availability, and remains in retry/replay/checkpoint
    history. The fake proves binding only; live normal/protected evidence must
    be authenticated by the OS/Guardian trust adapter respectively. No other
    revoker or override exists in 1.0.
12. Normal claim and protected Guardian claim are separate lanes. This module
    exposes no general protected consume operation.
13. Raw nonce/token/key/assertion bytes never enter persistent state,
    snapshots, logs, errors, or `Debug`.
14. Fake reads no hidden clock, random, environment, key, process, or I/O
    source and always emits `RuntimeKind::Fake`.

## Allowed Dependencies

- Rust standard library.
- `lattice-contracts` 1.5 immutable shared values.
- `lattice-cjson` 1.0 canonical-byte mechanics.
- Exact pinned `time` parsing/formatting only.

## Forbidden Dependencies

- Policy, Task Domain, Task Ledger, Project Registry, Writer Lease, ports,
  PostgreSQL/store, Workspace Git, Scope Check, Orchestrator, Review Runtime,
  Guardian, provider adapters, Codebase Memory, CLI/app layers, product
  repositories, or concrete I/O clients.

## Failure, Compatibility, And Migration

Unknown, malformed, missing, stale, expired, self-approved, cross-subject,
cross-project, wrong-lane, fake/live-mixed, reused, corrupt, or unsupported
input fails closed with stable typed errors or terminal denials. Denial never
partially changes authority state.

A future live verifier may add fixed algorithms and OS/key adapters without
accepting an `identity_verified` Boolean. A future PostgreSQL adapter must
reuse the public planner/verifier, enforce global nonce uniqueness, database
time, trusted checkpoint, and atomic claim/restart semantics, and must not
duplicate the state machine or hashes.

Protected release remains pending until the Guardian-only transaction
atomically rechecks receipt/head and DB time, consumes the nonce, appends
`ACTIVATION_CLAIMED`, and sets `DRAINING`.

## Acceptance Gates

| Gate | Evidence | Owner | Required for merge |
|---|---|---|---|
| Shared subject and receipt | fixed producer/version plus complete typed field matrix | Engineering | yes |
| Challenge/proof | domain-separated golden hashes and identity/trust substitution | Security review | yes |
| Nonce and retry | global reuse, exact retry, changed command, zero-partial mutation tests | Security review | yes |
| Time/currentness | before-issued, valid, expiry equality/after, historical/claimed head tests | Engineering | yes |
| Revocation | exact original approver, normal/protected, wrong actor/state/time, typed record, current-head loss, retry/replay/checkpoint tests | Security review | yes |
| Trust-lane isolation | normal/protected cross-product and Guardian identity/trust-root tests | Security review | yes |
| Replay/rollback | raw corruption, denied-tail chain, and trusted-checkpoint matrix | Engineering | yes |
| Policy composition | actual fake-owner receipt/current head plus full substitution matrix | Security review | yes |
| R3 fail closed | no caller review Boolean; missing Review Runtime owner denies | Architecture review | yes |
| Dependency/no-I/O/secrets | Cargo tree plus forbidden source and leak scans | Architecture review | yes |
| Full verification | workspace format, lint, Rust and preserved Node tests | Engineering | yes |

## Change Policy

Mission, subject schema, challenge/proof/nonce lifecycle, trust lanes,
idempotency, currentness/claim ownership, public hashes/receipts,
dependencies, or failure behavior changes require a versioned amendment,
SPEC/ADR trace, security and architecture review, and responsible-user
authorization.

## Amendment History

| Version | Date | Decision reference | Summary | Approver |
|---|---|---|---|---|
| 1.0 | 2026-07-29 | SPEC-002 v11, ADR-013, TASK-015 | Pure approval owner, deterministic fake, receipt/head, nonce/currentness, and claim split | User MVP-3 execution directive |
