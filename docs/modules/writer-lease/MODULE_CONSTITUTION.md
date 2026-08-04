---
module_id: writer-lease
name: Writer Lease
version: 1.0
status: active
owner: LATTICE maintainers
last_reviewed: 2026-07-29
---

## Mission

Own deterministic project-writer lease state, monotonic fencing allocation,
exact command receipts, expiry/suspect/recovery semantics, aggregate replay,
and a public pure transition boundary that future PostgreSQL persistence must
reuse.

## Non-Goals

- Decide Policy roles, capabilities, approvals, task transitions, or runtime
  admission transitions.
- Inspect/kill a process, read a clock, lead a daemon, or authenticate
  process/death/admission evidence.
- Perform worktree, Git, product-file, database, filesystem, process, network,
  provider, credential, payment, publication, or deployment I/O.
- Define physical tables, indexes, transactions, retries, or PostgreSQL
  permissions.
- Claim fake state is durable, concurrent, authenticated, or live authority.

## Owned Data

- `VACANT`, `ACTIVE`, and `SUSPECT` aggregate states.
- Complete lease identity and current authority projection.
- Signed-BIGINT-compatible fencing high-water mark and aggregate revision.
- Canonical command requests, transition records, terminal receipts, and
  untrusted/verified aggregate snapshots.
- Typed holder-death and replaced-leadership recovery evidence meaning.

## Public Contracts

- Plan one acquire, heartbeat, suspect, exact release, or evidence-bound revoke
  against one complete expected owner head without I/O.
- Apply only a verified plan whose exact precondition still matches.
- Verify and reconstruct an untrusted complete aggregate snapshot.
- Execute the same planner/verifier through a deterministic `RuntimeKind::Fake`
  in-memory owner.
- Return exact terminal receipts for both applied and denied commands.
- Return the current authority receipt/head only while a lease is active or
  suspect; vacant projects have no current authority head.

## Invariants

1. A project has at most one `ACTIVE` or `SUSPECT` product writer.
2. Identity binds project/snapshot/task/revision/spec/attempt, lease/holder/
   worktree/process-start, daemon instance/epoch, and fencing token.
3. Epochs, fencing tokens, and revisions are in `1..=i64::MAX`; allocation
   checks overflow before any mutation and never wraps, rolls back, or reuses.
4. Acquire and heartbeat require `ACTIVE` runtime admission. Draining permits
   exact release/recovery but not heartbeat. Canary and stopped admit no
   user-project transition. Reconciliation-required permits only typed
   recovery.
5. `observed_at >= expires_at` can mark an active lease suspect. Expiry alone
   never proves death and never revokes.
6. A suspect lease cannot heartbeat back to active.
7. Revoke requires exact holder-death or strictly newer daemon-leadership
   evidence bound to the current identity.
8. Exact command retry returns the identical terminal receipt before stale-head
   evaluation. Changed content under one command ID rejects permanently.
9. A denied command changes no lease state, fence, revision, transition, or
   authority head.
10. Every security-relevant authority-receipt field is mirrored into the
    independently queried full head. Receipt projection alone is not
    currentness.
11. Public untrusted replay rejects unknown versions, malformed values,
    reordering, truncation, duplication, orphan receipts, hash substitution,
    counter rollback, and claimed-state disagreement. Every terminal command
    receipt chains to its predecessor; aggregate command high-water and tail
    claims detect denial-only row loss.
12. The fake reads no clock/process/environment/random source and is visibly
    non-durable. All observations are explicit injected values.
13. Context-free replay proves internal consistency only. Restore and any
    rollback-sensitive acceptance require an independently retained,
    validated checkpoint binding project, command high-water/tail, and the
    complete snapshot digest.

## Allowed Dependencies

- Rust standard library.
- `lattice-contracts` 1.4 immutable shared values.
- `lattice-cjson` 1.0 canonical-byte mechanism.
- Exact pinned `time` parsing/formatting for canonical UTC timestamps.

## Forbidden Dependencies

- Task Domain, Policy, Project Registry, Task Ledger, ports, PostgreSQL/store,
  Workspace Git, Scope Check, Orchestrator, Approval Verifier, provider
  adapters, Codebase Memory, guardian, CLI/app layers, product repositories,
  or concrete I/O clients.

## Failure, Compatibility, And Migration

Unknown, missing, malformed, stale, cross-project, cross-task, mismatched,
expired, overflowed, unproven, corrupt, or unsupported input fails closed with
typed stable errors or terminal denial receipts. V1 ProjectLock is retained
only as characterization and local defense-in-depth until later removal; its
file/counter format is not imported or treated as V2 authority.

Step 6 may add a PostgreSQL adapter behind the public planner/verifier. It must
not change command/state/hash meaning and must prove current database time,
concurrent acquisition, restart/replay, stale connection fencing, and atomic
mutation admission before AC-05 can close. Command rows and the independently
protected checkpoint must advance atomically; the adapter must reconstruct the
checkpoint through the public validated constructor and may not derive it from
the untrusted history being verified.

## Acceptance Gates

| Gate | Evidence | Owner | Required for merge |
|---|---|---|---|
| State and command matrix | acquire/heartbeat/suspect/release/revoke/reacquire tests | Engineering | yes |
| Fence safety | zero/max/overflow/rollback/non-reuse matrix with zero partial mutation | Security review | yes |
| Exact retry | applied/denied retry and changed-content reuse tests | Engineering | yes |
| Recovery | expiry, PID/start identity, holder death, replaced epoch tests | Security review | yes |
| Replay | raw corruption/substitution, denied-tail truncation, receipt-chain, and trusted-checkpoint rollback matrix | Engineering | yes |
| Policy composition | actual fake owner receipt/current-head plus full substitution matrix | Security review | yes |
| Dependency and no-I/O | Cargo tree and forbidden-reference scan | Architecture review | yes |
| Full verification | workspace format, lint, Rust and preserved Node tests | Engineering | yes |

## Change Policy

Mission, state transitions, identity, recovery meaning, admission matrix,
fencing allocation, public hashes/receipts, dependencies, or failure behavior
changes require a versioned amendment, SPEC/ADR trace, security and
architecture review, and responsible-user authorization.

## Amendment History

| Version | Date | Decision reference | Summary | Approver |
|---|---|---|---|---|
| 1.0 | 2026-07-29 | SPEC-002 v10, ADR-012, TASK-014 | Pure Writer Lease owner, planner/verifier, fake, receipts, fencing, and recovery | User MVP-3 execution directive |
