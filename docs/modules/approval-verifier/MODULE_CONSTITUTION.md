---
module_id: approval-verifier
name: Approval Verifier
version: 1.4
status: active
owner: LATTICE maintainers
last_reviewed: 2026-08-28
---

## Mission

Own pure Rust, replayable approval-subject, challenge, proof, nonce-lifecycle,
availability, exact-command, and current-head semantics; issue fixed-owner
receipts through a deterministic non-durable fake; and provide the one
semantic core later reused by PostgreSQL and the protected Guardian claim.
Version 1.1 also owns the exact task/spec/budget-bound local execution
authority envelope consumed by the managed foreman. Version 1.2 accepts only
Policy's opaque typed decision evidence for that envelope; Policy evaluation
and fact loading remain Runtime composition responsibilities. Version 1.3
removes caller construction from the verified-approval lane: only an actual
verified receipt plus a separately loaded exact current head can issue or
reverify that authority. Version 1.4 adds a distinct owner-issued execution
challenge, proof, and binding receipt that commit the exact task/successor/spec/
budget subject without repurposing signer/authenticator evidence.

## Non-Goals

- Evaluate Policy, load Policy facts, decide risk/approval floors, or construct
  an `ExecutionGate`; the module may only inspect Policy's opaque typed outcome.
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
- Closed execution-authority source and capability, exact TaskSpec/successor/
  approval-subject/budget binding, validity window, evidence/receipt
  commitment, canonical digest, and strict untrusted-row verification.

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
- Construct a local reversible-execution authority only from a closed trusted
  policy result or an exact verified approval receipt. Never accept objective
  text or a caller Boolean as authority.
- For a closed-policy authority, consume the opaque `PolicyDecision` returned
  by Runtime's exact `ExecutionGate` evaluation, require its complete allowed
  result, and bind it to the supplied task/spec/budget and current-head facts.
  Never import or call `lattice_policy::evaluate`.
- For a verified-approval authority, require the complete owner-issued
  `ApprovalAuthorityReceipt`, an independently loaded exact current
  `ApprovalAuthorityHead`, and the matching task/successor/spec/subject/budget
  inputs. A public digest-only constructor does not exist.
- Require an `ExecutionApprovalChallenge` signed through the normal owner lane
  and an `ExecutionApprovalBindingReceipt` linked to the ordinary owner receipt.
  The base receipt keeps its independent signer/authenticator `evidence_digest`;
  the ordinary `ApprovalSubject::Execution` Task Spec field alone cannot
  authorize a caller-selected task reference, successor stream, or budget.
- Issue that binding receipt only as the terminal result of the owner aggregate's
  append-only `BIND_EXECUTION` command against the exact verified-available
  head. The standalone receipt constructor is private. Exact command retry is
  replayable; a changed proof, subject, head, or second different binding is a
  durable denial. Snapshot, checkpoint, and canonical-byte restore retain the
  command, terminal receipt, and binding receipt or fail closed on tamper.

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
12. An execution binding is owner state, not caller context. Its command,
    execution challenge/proof, base approval receipt, expected current head,
    terminal outcome, and binding receipt participate in the aggregate hash,
    strict snapshot replay, and trusted checkpoint. A legacy snapshot without
    this command may still replay as historical approval state, but it cannot
    issue managed execution authority.
13. Normal claim and protected Guardian claim are separate lanes. This module
    exposes no general protected consume operation.
14. Raw nonce/token/key/assertion bytes never enter persistent state,
    snapshots, logs, errors, or `Debug`.
15. Fake reads no hidden clock, random, environment, key, process, or I/O
    source and always emits `RuntimeKind::Fake`.
16. The managed execution capability grants only bounded reversible local task
    execution. It cannot authorize merge, push, default-branch mutation,
    deploy, release, payment, external messages, or permanent deletion.
17. `VERIFIED_APPROVAL` requires an approval-receipt digest;
    `CLOSED_POLICY_NO_APPROVAL_REQUIRED` forbids one. Every authority binds an
    exact task reference, successor stream, TaskSpec, approval subject, budget,
    evidence digest, and finite canonical UTC validity interval.
18. A receipt digest, structural `receipt.head()`, caller currentness Boolean,
    or previously persisted execution-authority row cannot mint or refresh a
    verified-approval authority. Current receipt/head equality and validity are
    checked before construction; restart replays historical evidence without
    signing it again under a changed process identity.
19. Legacy normal receipts without the execution-specific owner binding receipt,
    and legacy Policy results without an exact managed execution binding, fail
    closed. Compatibility never reconstructs missing authority from caller
    fields or replaces signer/authenticator evidence with a subject digest.

## Allowed Dependencies

- Rust standard library.
- `lattice-contracts` 1.5 immutable shared values.
- `lattice-cjson` 1.0 canonical-byte mechanics.
- `lattice-policy` bounded immutable decision-evidence types only:
  `ExecutionGateDecisionEvidence`, `PolicyDecision`, `PolicyEvidence`,
  `DecisionKind`, and `DecisionStage`.
- Exact pinned `time` parsing/formatting only.

## Forbidden Dependencies

- Policy evaluation/subject/fact APIs (including `evaluate`), Task Domain,
  Task Ledger, Project Registry, Writer Lease, ports,
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
| Managed execution authority | task/spec/budget binding, receipt/source separation, expiry, external-effect denial, and tamper replay | Security review | yes |

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
| 1.1 | 2026-08-26 | SPEC-011, ADR-028 | Add exact local reversible-execution authority envelope without broadening approval lanes | Delegated product owner |
| 1.2 | 2026-08-27 | Phase 4 formal execution-authority review | Keep exact Policy evaluation/current fact loading in Runtime; Approval Verifier consumes only opaque bounded decision evidence and cannot self-sign from hashes | Delegated product owner |
| 1.3 | 2026-08-27 | SPEC-011, ADR-028 durable-core review | Remove digest-only verified-approval construction; require an actual owner receipt plus independent current head and preserve historical restart evidence without re-signing | Delegated product owner |
| 1.4 | 2026-08-28 | SPEC-011, ADR-028 execution-authority security repair | Add append-only owner `BIND_EXECUTION` challenge/proof/binding receipts with strict snapshot/checkpoint replay while preserving signer evidence, and reject unbound Policy/receipt replay | Delegated product owner |
