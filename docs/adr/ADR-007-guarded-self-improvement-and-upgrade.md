# ADR-007: Guarded Self-Improvement and A/B Upgrade

- Status: accepted; approved by user on 2026-07-29
- Date: 2026-07-29
- Decision owner: user
- Related: SPEC-002, ADR-001, ADR-002, ADR-005

## Context

The user wants an AI development platform that continuously improves and
iterates on their computer. Allowing the active process to edit, approve, and
overwrite itself would combine proposer, implementer, reviewer, releaser, and
rollback authority in one failure domain.

Self-improvement must therefore be a controlled product workflow, not a bypass
around the product workflow.

## Proposed Decision

Separate two loops:

### Improvement loop

```text
observe -> propose -> freeze Task Packet -> implement in isolated worktree
-> verify -> read-only review -> produce immutable release candidate
```

- Observations include task outcomes, user corrections, failures, performance,
  retrieval quality, and adapter capability drift.
- Hermes may suggest a candidate; it cannot approve, persist authoritative
  memory, or implement it.
- Codex remains the sole product-code Implementer.
- Improvements to policy, constitutions, the guardian, credentials, network,
  database compatibility, or protected capabilities require human approval.

### Activation loop

```text
inactive slot -> manifest verification -> read-only shadow replay
-> exact release approval -> recoverable guardian activation -> health window
-> finalize or rollback
```

- Release slots are immutable A/B bundles.
- Each manifest records source commit/tree, dependency lock, binaries,
  migration checksums/compatibility, policy/capability delta, and verification
  evidence hashes.
- The active bundle is never overwritten in place.
- A separate minimal supervisor owns activation, stop, health, and rollback.
  It cannot edit product code, approve policy, or accept ordinary tasks.
- `latticed` records the candidate and promotion request. The guardian becomes
  the sole writer to a dedicated PostgreSQL release stream through audited
  stored procedures; that narrow role can also activate/retire daemon epochs
  but has no task, memory, product-project, or general SQL write authority.
- A shadow candidate uses a real read-only database role and cannot append
  events or acquire a product writer lease.

## Protected Approval Trust Chain

Release approval is verified by an `approval-verifier` contract independent of
the candidate and normal task gateway.

The exact approval subject binds:

- authenticated actor identity, authority, channel, and session;
- release ID/revision and Task Spec hash;
- manifest digest, source commit/tree, dependency lock, every binary digest,
  and every migration checksum;
- schema compatibility, policy delta, capability delta, and target slot;
- requested target epoch, one-use nonce, `issued_at`, and `expires_at`.

The initial hardened version uses a guardian-owned, OS-authenticated local
approval surface or service identity with a trust root unavailable to the
candidate and normal `latticed` process. OpenClaw may initiate and display the
request, but an ordinary OpenClaw session or possession of the normal task IPC
token cannot alone satisfy protected release approval.

The guardian verifies the signed/MAC-bound receipt, exact manifest digest,
actor authority, expiry, and subject hash before claiming activation.
`approval-verifier` owns cryptographic/identity validation; it does **not**
consume the nonce independently. The guardian-only `claim_activation`
transaction is the sole nonce consumer: it locks the release/nonce, verifies
receipt and subject hashes plus database-clock expiry, marks the nonce used,
appends `ACTIVATION_CLAIMED`, and changes runtime admission to `DRAINING`
atomically. Until the separate OS identity, ACL, trust root, and claim path are
live-tested, release promotion is disabled and this separation is classified
documented-only.

TASK-015/ADR-013 freezes the pure/fake predecessor of this trust chain.
Approval Verifier may issue a visibly fake protected-pending-claim receipt and
test exact subject, nonce, time, replay, and current-head semantics, but exposes
no general protected consume command. It cannot establish independent review
acceptance. Protected activation remains disabled until Review Runtime
authority and the Guardian/PostgreSQL atomic claim path both pass their direct
live gates.

## Recoverable Activation Saga

Activation is a durable saga rather than one cross-resource transaction:

```text
STAGED
-> PROMOTION_APPROVED
-> ACTIVATION_CLAIMED
-> OLD_DAEMON_DRAINED
-> SLOT_POINTER_WRITTEN
-> CANDIDATE_STARTED
-> EPOCH_ACTIVATED
-> HEALTH_WINDOW_PASSED
-> ACTIVATION_FINALIZED

failure after claim:
-> ROLLBACK_REQUESTED
-> ROLLBACK_SLOT_WRITTEN
-> ROLLBACK_DAEMON_STARTED
-> ROLLBACK_EPOCH_ACTIVATED
-> ROLLED_BACK

ambiguous/unrecoverable evidence:
-> RECONCILIATION_REQUIRED
```

- The guardian appends intent before every external step and outcome after it.
- `claim_activation` atomically consumes the exact approval nonce, appends the
  claim, and sets PostgreSQL runtime admission to `DRAINING`. New tasks, new
  leases/effect claims, and new user-project side effects then fail closed.
- `OLD_DAEMON_DRAINED` requires evidence of zero valid product writer leases,
  zero claimed/running or unknown external effects, every active effect
  reconciled, and termination of every writable Codex child/process tree.
  Task submission may remain queued, but cannot be admitted.
- Release procedures compare expected activation ID/state and reject skipped,
  repeated, or stale transitions.
- The boot pointer is a checksum-bound projection containing activation ID,
  manifest digest, slot ID, and expected daemon epoch.
- On guardian restart, PostgreSQL wins. If the durable state precedes a pointer
  write, a stray pointer is restored to the last finalized slot. If durable
  state confirms the write but the pointer is missing/stale, the guardian
  rewrites it only after re-verifying the immutable bundle. Ambiguous evidence
  enters `RECONCILIATION_REQUIRED`; no daemon receives active-writer status.
- `CANDIDATE_STARTED` is initially read-only. `EPOCH_ACTIVATED` atomically
  assigns a higher, non-wrapping epoch to its instance and changes admission to
  `CANARY`. Except for guardian-only release/epoch procedures, every
  daemon-authorized durable mutation checks instance, epoch, and admission in
  the same transaction; the old instance is rejected even over an existing
  database connection.
- `CANARY` permits only the guardian-reserved health stream. No user-project
  task, lease/effect, registry, artifact publication, memory, review,
  capability, or ordinary approval mutation is allowed.
- `ACTIVATION_FINALIZED` is the only transition that changes admission from
  `CANARY` to `ACTIVE`. A failed/ambiguous canary enters rollback or
  `RECONCILIATION_REQUIRED`, never normal work.
- Rollback starts the prior compatible binary as a new instance with a still
  higher epoch in `CANARY`, runs the same reserved canary, and changes admission
  to `ACTIVE` only when `ROLLED_BACK` is finalized. Epochs are never decremented
  or reused.

## Health Contract

- Shadow checks prove start, replay, schema read, and query behavior only.
- After epoch activation, the candidate must append and replay a canary on a
  guardian-reserved system stream through the normal epoch-checked store path.
  It cannot touch a user project.
- The guardian owns an immutable minimum health contract, timeout, sampling
  window, rollback threshold, and disagreement rule. A candidate cannot lower
  these requirements.
- Failed replay, startup, canary, crash-loop, or compatibility checks trigger
  the rollback saga. An ambiguous result fails closed.

## Database Migration Rule

- The first A/B MVP forbids every schema migration during activation. Active A,
  shadow B, candidate B, and rollback A must already support the same schema.
- A later version may add an expansion-only sequence:
  `MIGRATION_PLANNED -> COMPATIBILITY_PROVEN_A_AND_B -> EXPANSION_APPLIED
  -> SHADOW_VERIFIED -> ACTIVATION`.
- That future path needs a single migration owner/lock, intent/outcome events,
  interruption recovery, and evidence that A and B both remain compatible.
- Drop, rename, destructive rewrite, new non-null requirements, or other
  contract changes are separate human-approved migrations.
- Automatic rollback switches binaries; it never guesses a destructive data
  downgrade. Backup/restore evidence is required before destructive migration
  is considered.

## Promotion Policy

The initial hardened version requires user approval for LATTICE core promotion.
A later policy may allow automatic promotion only for pre-authorized,
low-risk, local, reversible changes that do not alter schema, policy,
constitutions, supervisor, credentials, network access, external cost, or
capabilities.

Preparing, testing, shadowing, and reporting a candidate may be automated.
Silence or lack of response is never promotion approval.

## Stop And Recovery

- The user can stop proposal generation, task execution, or upgrade activation.
- The guardian remains independently startable for status/stop/rollback.
- Repeated crash loops preserve the last compatible slot and evidence.
- If database truth and the boot projection disagree, the saga reconciliation
  rules repair only a uniquely proven state; otherwise activation fails closed
  at `RECONCILIATION_REQUIRED` with runtime admission set to
  `RECONCILIATION_REQUIRED`, which permits no daemon mutation or effect.

## Consequences

- Continuous improvement remains useful while avoiding self-approval.
- The separate guardian and two release slots add packaging, compatibility,
  and recovery-test work.
- Binary rollback is practical; destructive database rollback remains
  intentionally outside automatic behavior.
- OS-admin-equivalent malicious processes are not contained by hashes or
  in-process policy and require a future verified OS isolation boundary.

## Approval Gate

Accepting this ADR authorizes detailed module constitutions and tickets. It
does not authorize an automatic promotion policy, installation, live update,
service restart, file replacement, database migration, or deployment.
