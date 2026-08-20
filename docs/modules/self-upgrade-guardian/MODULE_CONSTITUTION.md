---
module_id: self-upgrade-guardian
name: Self-Upgrade Guardian
version: 1.0
status: active
owner: LATTICE maintainers
last_reviewed: 2026-08-20
---

## Mission

Own the separate minimal protected composition boundary that verifies immutable
release candidates and coordinates recoverable A/B activation, health,
reconciliation, and rollback without granting the active daemon authority to
approve or replace itself.

## Non-Goals

- Write product code, accept ordinary tasks, run the normal Gateway, or become
  a second Orchestrator.
- Approve policy, specifications, constitutions, reviews, credentials, or the
  Guardian's own replacement.
- Own normal approval claims, Writer Lease semantics, Artifact lifecycle,
  project/task/memory truth, or general PostgreSQL mutation.
- Perform a destructive or activation-time database migration.
- Overwrite the active release bundle in place or treat a filesystem pointer as
  durable truth.
- Turn a candidate, active daemon, OpenClaw session, task token, caller Boolean,
  or ordinary OS-user lane into protected release authority.

## Owned Data

- Immutable local A/B slot identities and verified bundle bindings.
- Checksum-bound boot-pointer projection and its external-step intent/outcome
  evidence.
- Activation and rollback saga ordering, Guardian-owned minimum health
  contract, sampling window, timeout, and disagreement disposition.
- Guardian recovery and rollback receipts.

PostgreSQL remains the durable truth for release state, runtime admission, and
daemon epochs through narrow Guardian-only procedures. Approval Verifier owns
subject/proof/nonce semantics; Writer Lease, Task Ledger, Artifact Store,
Project Registry, Policy, and their repositories retain their existing data and
mutation ownership.

## Public Contracts

- Inspect slot metadata and verify one complete immutable release manifest,
  dependency lock, binary digests, empty migration set, schema compatibility,
  source/target distinction, and approval subject binding.
- Start and stop a read-only shadow candidate and report bounded shadow health.
- Through the separately OS-authenticated Guardian surface, atomically
  `claim_activation` using the exact protected-pending approval.
- Prove complete drain, append release-step intents/outcomes, project the boot
  pointer, start a candidate read-only, activate a higher daemon epoch, run the
  reserved write canary, finalize, request rollback, and reconcile.
- Return exact retry receipts and typed fail-closed denial or
  `RECONCILIATION_REQUIRED` evidence.
- Expose read-only recovery status to the normal operator surface without
  exposing protected claim, slot, admission, epoch, stop, or rollback authority.

Only the protected Guardian composition root may invoke mutating contracts.
Normal Gateway and candidate callers may request/display status but cannot
satisfy or delegate protected authority.

## Invariants

1. The active bundle is never overwritten in place; source and target slots are
   distinct immutable identities.
2. Only the independently authenticated Guardian changes the active-slot
   projection or invokes release/admission/epoch procedures.
3. Shadow candidates are read-only and cannot append control events, acquire a
   product lease, or mutate a user project.
4. Every external saga step has a durable intent before the effect and a
   durable outcome after it; unknown outcomes never imply success.
5. Protected claim atomically rechecks the exact approval receipt, subject,
   database time, nonce, and current release state; consumes the nonce; appends
   `ACTIVATION_CLAIMED`; and sets admission to `DRAINING` in one transaction.
6. `OLD_DAEMON_DRAINED` requires zero valid product leases, zero
   claimed/running/unknown effects, reconciled outcomes, and no writable Codex
   child or process tree.
7. Daemon epochs are positive, non-wrapping, strictly increasing, never reused,
   and rechecked in the same transaction as every daemon-authorized mutation.
8. `EPOCH_ACTIVATED` sets `CANARY`; only the Guardian-reserved system health
   stream may be written until exact finalization changes admission to `ACTIVE`.
9. A candidate cannot weaken the Guardian-owned minimum health contract,
   timeout, sampling window, crash threshold, or disagreement rule.
10. Rollback uses a verified schema-compatible prior slot, starts a new
    instance, assigns an epoch greater than the failed activation, and passes
    the same write canary before `ACTIVE`.
11. PostgreSQL wins over the boot projection. Ambiguous database/pointer,
    process, bundle, epoch, or health evidence enters
    `RECONCILIATION_REQUIRED`, which admits no daemon mutation or effect.
12. The first A/B version executes no schema migration during activation or
    rollback.

## Allowed Dependencies

- `lattice-contracts` for neutral immutable identities and evidence values.
- `lattice-cjson` for canonical byte mechanics only.
- Approval Verifier's public protected-pending subject/proof/currentness
  verifier; never its private state or a general consume command.
- Narrow typed ports for read-only task/lease/effect/process status and
  Guardian-only release, runtime-admission, daemon-epoch, and reserved-canary
  procedures.
- Read-only immutable Artifact bundle/manifest evidence.
- Bounded process/service control, cryptographic manifest verification, and an
  independently provisioned OS peer-identity/trust adapter.

## Forbidden Dependencies

- Product source/worktree editing, Git implementation operations, normal task
  orchestration, provider/model execution, or general Gateway/OpenClaw task
  IPC.
- General PostgreSQL credentials, arbitrary SQL, direct domain-table access,
  or another durable release/epoch truth.
- Policy mutation, constitution approval, normal approval consumption, Artifact
  lifecycle mutation, task/project/memory/review/capability streams, or product
  Writer Lease acquisition.
- Caller-selected filesystem paths, service commands, credentials, slots,
  migrations, health thresholds, epochs, or recovery disposition.

## Failure, Compatibility, And Migration

Unknown, missing, stale, expired, replayed, substituted, cross-bound, malformed,
overflowed, incompatible, partially committed, or disagreement evidence fails
closed before the next effect. Exact same-command retry may replay an identical
terminal receipt; changed content, skipped state, duplicate first effect, stale
expected state, or epoch mismatch denies deterministically.

The initial activation contract requires active, candidate, and rollback
bundles to support the same database schema and an empty migration set. A later
expansion-only migration sequence requires a separate versioned amendment,
single migration owner, interruption recovery, A/B compatibility evidence, and
responsible-user approval. Automatic rollback switches binaries only and never
guesses a destructive data downgrade.

## Acceptance Gates

| Gate | Evidence | Owner | Required for merge |
|---|---|---|---|
| Closed state/transition contract | exact legal/denied transition and retry matrix | Engineering | yes |
| Protected surface | disposable OS identity, ACL, trust-root and normal-lane substitution tests | Security review | yes |
| Atomic claim/admission | concurrent claim, expiry, replay, acknowledgement-loss and restart tests | Engineering | yes |
| Immutable slots and pointer recovery | corrupt/alias/partial-pointer/power-loss/reconciliation drills | Engineering | yes |
| Complete drain | lease/effect/unknown-outcome/writable-child matrices | Security review | yes |
| Epoch and canary fencing | stale live connection, user-write denial, reserved append/replay and higher-epoch tests | Security review | yes |
| Health and rollback | crash-loop, disagreement, incompatible prior slot and higher-epoch rollback drills | Engineering | yes |
| Dependency and authority direction | Cargo tree, fixed-procedure/ACL closure and forbidden-reference scan | Architecture review | yes |
| Full readiness | focused/full tests, disposable PostgreSQL/process/filesystem drills, independent reviews and commit-bound CI | Engineering and responsible user | yes |

## Change Policy

Mission, non-goals, ownership, protected trust lane, release/admission/epoch
authority, state transitions, slot/health/rollback invariants, public contracts,
dependency direction, failure behavior, migration policy, or acceptance gates
require a versioned amendment, SPEC/ADR trace, security and architecture review,
and explicit responsible-user approval. A candidate or active Guardian may not
amend this constitution to authorize its own activation.

## Amendment History

| Version | Date | Decision reference | Summary | Approver |
|---|---|---|---|---|
| 1.0 | 2026-08-20 | SPEC-002 v27, ADR-007, TASK-045, approved V2 amendment | Activate the separate protected A/B activation, health, reconciliation, and rollback boundary without implementation or live authority claims | User MVP-3 execution directive |
