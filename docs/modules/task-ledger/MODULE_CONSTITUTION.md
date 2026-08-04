---
module_id: task-ledger
name: Task Ledger
version: 2.1
status: active
owner: LATTICE maintainers
last_reviewed: 2026-08-02
---

## Mission

Own the versioned event, hash-chain, exact command-receipt, verified replay,
resource-projection, effect-intent outbox-admission, pure append-plan, and
complete checkpoint semantics that PostgreSQL persists as the single durable
control-plane truth.

## Non-Goals

- Decide Policy or legal Task Domain transitions.
- Persist files or database rows, claim/deliver an outbox effect, or supervise
  a provider. The module may derive one immutable outbox-admission record from
  a successfully appended `EFFECT_INTENT` with audit outcome `RECORDED`; it
  performs no I/O.
- Allocate a clock value, random event ID, daemon epoch, approval, writer
  lease, or runtime admission.
- Authenticate a caller, owner lookup, or database connection.
- Accept arbitrary authoritative JSON, raw secrets, credentials, environment
  dumps, prompts, or external output.
- Provide a second durable truth or distributed consensus.

## Owned Data

- Complete task-stream identity and stream ID semantics.
- Versioned command request, event, stream-head, command-receipt, resource
  projection, observation, and receipt hash subjects.
- Closed event kinds/outcomes and bounded non-authoritative diagnostics.
- Exact command idempotency and terminal append/deny receipt meaning.
- Verified stream replay and Ledger-owned resource-counter projection.
- Immutable outbox-admission meaning for a successfully appended
  `EFFECT_INTENT` with audit outcome `RECORDED`, including its stable
  intent/admission commitments.
- Complete deterministic Ledger checkpoint semantics across identity, head,
  projection, events, every terminal command including denials, and admitted
  outbox intents.

The TASK-013 fake owns only disposable process-memory test state. Postgres
Store 1.3 owns physical durable rows, locks, transactions, indexes, projection
persistence, and outbox-admission persistence without acquiring event meaning.
Outbox claim, delivery, retry, and provider reconciliation remain deferred
until a separately approved module/ticket owns those mechanics.

## Public Contracts

- Construct one task stream from exact project/snapshot/task/revision/spec-hash
  and accounting-currency identity.
- Produce and verify a full zero or non-zero stream head.
- Append one typed event against an exact full expected head and stable command
  ID.
- Return the identical terminal receipt for the same stream/command/request;
  reject command-ID reuse with another request.
- Deny a new stale/mismatched head or overflow without event/head/resource
  mutation.
- Export explicitly untrusted event plus full command-key/request/receipt
  records and verify them through one pure persistence replay boundary.
- Verify all event, receipt, predecessor, projection, and claimed-head fields
  during replay.
- Issue a fixed-producer fake resource observation bound to the current full
  stream head and exact effect claim.
- Validate a resource receipt against the current fake owner state before
  producing a full current-head projection for Policy composition.
- Create a structural Fake or Live vacant verified stream without claiming
  that a structural Live value is authenticated or durable.
- Plan one append against a complete verified stream and apply the indivisible
  plan only when the complete base checkpoint is unchanged.
- Return verified retained appended or denied command receipts after replay.
- Export complete untrusted command/event/outbox rows and verify them against
  one independently retained checkpoint.

## Invariants

1. A naked Task ID is never a stream identity.
2. Sequence zero has the zero predecessor/event digest; sequence begins at one
   and advances by exactly one without wrapping.
3. Every request/event/head/receipt/resource hash uses its own frozen domain.
4. Every event hash covers the complete sanitized event, predecessor, and
   resulting resource projection.
5. Exact retry lookup precedes stale-head evaluation and returns a byte-equal
   receipt after later stream advancement.
6. Reused command ID with changed content never appends or returns the old
   result.
7. Unknown schema/event versions, corruption, truncation, reorder, duplicates,
   orphan receipts, and head/projection disagreement fail closed.
8. A durable command record retains its complete canonical request source;
   digest-only denied receipts are insufficient for verified replay.
9. Authoritative fields are typed; diagnostic text is bounded, sanitized,
   non-authoritative, and never carries a legal transition or effect identity.
10. Resource counters derive only from verified resource events. A receipt
   projected from itself does not prove currentness.
11. Task-state legality remains exclusively owned by Task Domain and future
    Orchestrator composition.
12. The fake always identifies as `RuntimeKind::Fake` and cannot claim
    durability, authenticated authority, or live effect admission.
13. Fake and live persistence use the same pure append planner; an adapter
    cannot construct event/head/receipt/outbox fragments independently.
14. Only an appended `EFFECT_INTENT` whose audit outcome is `RECORDED` produces
    one outbox admission. Its intent digest equals the event subject digest;
    existing non-`RECORDED` effect-intent combinations remain valid appended
    events but produce no admission, and denied/non-effect commands produce
    none.
15. Every terminal command, including a denial that leaves the event head
    unchanged, changes the complete checkpoint exactly once. Exact retry does
    not change it.
16. Checkpoint command ordering is canonical `(stream_id, command_id)` order,
    not database or caller vector order; event ordering is exact sequence and
    outbox ordering is exact admitted event sequence.
17. An internally valid snapshot is not current until its complete checkpoint
    equals an independently retained trusted commitment.
18. Domain sequence/resource values retain the full `u64` range. A persistence
    adapter must not silently narrow them to signed `BIGINT`.

## Allowed Dependencies

- Rust standard library.
- `lattice-contracts` 1.9 immutable shared values and Task Ledger receipt/head
  representations, whose Ledger-specific semantics remain unchanged from 1.3.
- `lattice-cjson` 1.0 canonical-byte/hash mechanics only.
- exact `time` 0.3.54 parsing/formatting only for caller-supplied canonical UTC
  RFC 3339 timestamps; no clock reads.

## Forbidden Dependencies

- Task Domain, Policy Engine, ports, Project Registry, Writer Lease, Approval
  Verifier, Orchestrator, concrete adapters/stores, filesystem/Git/database/
  process/network/environment/randomness clients, model SDKs, credentials, and
  product repositories.

## Failure, Compatibility, And Migration

Validation and replay return stable typed errors and never repair, delete, or
silently skip corrupt/unknown records. V1 files and hashes remain read-only
characterization evidence; V1 Node hashing, arbitrary payloads, filesystem
persistence, Task Domain projection import, and unknown-event no-op behavior
are not active V2 compatibility.

The in-memory fake is not restart evidence. Task Ledger 2.1 provides the pure
planner/checkpoint/replay boundary used by PostgreSQL, but does not itself
claim durable currentness. Postgres Store 1.3 must atomically commit the
command receipt, optional event, head, projection, outbox admission, checkpoint,
and physical Store receipt under daemon epoch/runtime-admission checks. Unknown
commit retries the same command/request and must reconstruct the identical
retained plan/receipt.

## Acceptance Gates

| Gate | Evidence | Owner | Required for merge |
|---|---|---|---|
| Hash/head/replay | domain fixtures plus tamper/reorder/truncate/unknown matrices | Engineering | yes |
| Idempotency/atomicity | exact retry, mutation, stale head, overflow, no-partial-mutation tests | Engineering | yes |
| Diagnostic safety | bounds, NFC, secret redaction/rejection, Debug/error leak tests | Security review | yes |
| Resource ownership | projection, receipt/current-head, historical/substitution tests | Security review | yes |
| Pure plan parity | fake uses the same vacant/plan/apply boundary as live persistence; existing hashes remain unchanged | Engineering | yes |
| Durable checkpoint | denial tail, event/command/outbox order, rollback, injection, and wrong-checkpoint matrices | Security review | yes |
| Outbox admission | exactly one admission for appended `EFFECT_INTENT` + `RECORDED`; none for appended non-`RECORDED`, denied, or non-effect commands | Engineering | yes |
| Dependency/no-I/O boundary | Cargo tree and forbidden-reference scan | Architecture review | yes |
| Full verification | workspace format, lint, Rust, and preserved Node tests | Engineering | yes |

## Change Policy

Stream identity, event/receipt/resource field selection, hash domains,
idempotency order, replay/corruption behavior, diagnostic rules, resource
counter meaning, or dependency direction requires a versioned amendment, ADR,
compatibility plan, security and architecture review, and user authorization.

## Amendment History

| Version | Date | Decision reference | Summary | Approver |
|---|---|---|---|---|
| 1.0 | 2026-07-29 | ADR-001, TASK-002 | Initial Node/file Task Ledger | Current user task |
| 2.0 | 2026-07-29 | SPEC-002 v9, ADR-005/008/011, TASK-013 | Pure Rust event/receipt/replay/resource semantics plus visibly non-durable fake; PostgreSQL persistence deferred | User MVP-3 execution directive |
| 2.1 | 2026-08-02 | SPEC-002 v23, ADR-019, TASK-021 | Shared pure vacant/plan/apply boundary, verified retained receipts, effect-intent outbox admission, and independent complete Ledger checkpoint without adding I/O | Approved V2 amendment and user MVP-3 execution directive |
