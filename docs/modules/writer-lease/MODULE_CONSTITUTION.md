---
module_id: writer-lease
name: Writer Lease
version: 1.1
status: active
owner: LATTICE maintainers
last_reviewed: 2026-08-09
---

## Mission

Own deterministic project-writer lease state, monotonic fencing allocation,
exact command receipts, expiry/suspect/recovery semantics, canonical aggregate
snapshot/checkpoint bytes, pure replay, and the repository contract that every
live persistence adapter must reuse.

## Non-Goals

- Decide Policy roles, capabilities, approvals, task transitions, or runtime
  admission transitions.
- Inspect/kill a process, read a clock, lead a daemon, or authenticate
  process/death/admission evidence.
- Perform worktree, Git, product-file, database, filesystem, process, network,
  provider, credential, payment, publication, or deployment I/O.
- Define physical tables, indexes, transactions, driver retries, extension
  installation, or PostgreSQL permissions.
- Claim fake state is durable, concurrent, authenticated, or live authority.

## Owned Data

- `VACANT`, `ACTIVE`, and `SUSPECT` aggregate states.
- Complete lease identity and current authority projection.
- Signed-BIGINT-compatible fencing high-water mark and aggregate revision.
- Canonical command requests, transition records, terminal receipts, and
  untrusted/verified aggregate snapshots.
- Typed holder-death and replaced-leadership recovery evidence meaning.
- Canonical complete untrusted aggregate snapshot bytes and independently
  protected checkpoint bytes, including their exact version/hash domains.
- The abstract `WriterLeaseRepository` execute/current-authority/assert-current
  contract, the domain replay summary that distinguishes absent, active, and
  released project history, and component-free repository errors. A concrete
  adapter owns all I/O and physical persistence provenance; the domain summary
  alone does not attest that its source was PostgreSQL.

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
- Encode and parse one bounded canonical snapshot/checkpoint representation;
  parsing establishes shape only, while public replay plus an independently
  retained checkpoint establishes verified current state.
- Expose a repository trait that atomically executes one high-level typed
  command, loads one replay-verified current authority, and asserts one exact
  independently supplied current head. The trait grants
  no SQL, client, table, credential, migration, or caller-created receipt
  surface.
- Expose a bounded domain replay summary with project identity, optional current
  authority, fencing high-water, transition high-water, and command high-water.
  Concrete repositories may return it only after their own physical replay;
  `None` and a released `Some { current_authority: None, ... }` are distinct.

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
14. Snapshot and checkpoint bytes are canonical, versioned, bounded, secret-
    free, and complete enough for context-free replay. Unknown fields,
    truncation, reorder, duplicate records, or trailing bytes fail closed.
15. Repository implementations use the same public planner/replay/checkpoint
    boundary as the fake. A repository cannot construct a transition, receipt,
    authority head, snapshot, or checkpoint independently.
16. A live repository result is current only when its independently loaded
    project head/checkpoint matches the verified replay. Persistence evidence
    never changes lease semantic meaning.

## Allowed Dependencies

- Rust standard library.
- `lattice-contracts` 1.12 immutable shared values; Writer Lease receipt/head
  semantics remain compatible with their 1.4 introduction.
- `lattice-cjson` 1.0 canonical-byte mechanism.
- Exact pinned `time` parsing/formatting for canonical UTC timestamps.
- Minimal component-free repository error/value support from the Rust standard
  library; no concrete I/O type enters the public trait.

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

Version 1.1 adds canonical snapshot/checkpoint bytes and an abstract repository
trait without adding I/O or changing command/state/hash meaning. PostgreSQL
Writer Lease 1.0 implements that trait as a separate adapter and must prove
current database time, concurrent acquisition, restart/replay, stale
connection fencing, monotonic non-reuse, and atomic mutation admission before
formal writer acceptance can close. Command rows, fencing high-water,
snapshot, and independently protected checkpoint must advance atomically; the
adapter reconstructs the checkpoint through the public validated constructor
and may not derive it from the untrusted history being verified.

## Acceptance Gates

| Gate | Evidence | Owner | Required for merge |
|---|---|---|---|
| State and command matrix | acquire/heartbeat/suspect/release/revoke/reacquire tests | Engineering | yes |
| Fence safety | zero/max/overflow/rollback/non-reuse matrix with zero partial mutation | Security review | yes |
| Exact retry | applied/denied retry and changed-content reuse tests | Engineering | yes |
| Recovery | expiry, PID/start identity, holder death, replaced epoch tests | Security review | yes |
| Replay | raw corruption/substitution, denied-tail truncation, receipt-chain, and trusted-checkpoint rollback matrix | Engineering | yes |
| Snapshot bytes | canonical golden bytes plus malformed/version/bound/truncation/reorder/trailing-data matrices | Engineering | yes |
| Repository contract | fake/conformance implementation proves exact execute/current-authority/assert-current, planner parity, retry, and component-free failures | Architecture review | yes |
| PostgreSQL repository | concurrent acquire, restart/replay, stale fence, atomic checkpoint/high-water, and unknown-outcome reconciliation | Integration review | yes for production use |
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
| 1.1 | 2026-08-09 | SPEC-003 v3, ADR-023, TASK-038 | Canonical complete snapshot/checkpoint bytes and the sole abstract repository contract used by PostgreSQL persistence | User TASK-038-first direction |
