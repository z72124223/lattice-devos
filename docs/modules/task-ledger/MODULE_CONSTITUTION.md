---
module_id: task-ledger
name: Task Ledger
version: 2.5
status: active
owner: LATTICE maintainers
last_reviewed: 2026-08-24
---

## Mission

Own the versioned event, hash-chain, exact command-receipt, verified replay,
resource-projection, effect-intent outbox-admission, pure append-plan, and
complete checkpoint semantics, including the closed autonomy-receipt event,
and the authoritative Task-created autonomy profile discriminator that
PostgreSQL persists as the single durable control-plane truth.

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
- Schedule or execute autonomous work, call a model/provider, perform Git or
  filesystem effects, or infer authority from an autonomy receipt.

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
- The closed `AutonomyReceiptRecorded` event and its fixed-scalar canonical
  subject/authority-digest meaning. The module owns this event's semantics,
  ordering, hash participation, and replay; it does not own durable rows.
- The `lattice.task-ledger.task-created-profile/1.0` discriminator carried by
  the authoritative `TASK_CREATED.action` field for bounded task-control
  streams. The legacy `CONTROLLED_CODEX_CANARY` value is receipt-optional;
  `CONTROLLED_CODEX_CANARY_AUTONOMY_V1` requires exactly one V1 receipt.
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
- Plan and verify the optional closed autonomy-receipt event without adding
  arbitrary payloads or a public MCP field. A historical profile may omit it;
  a new autonomy-enabled task requires exactly one after `TASK_CREATED` and
  before any writable or external effect.
- Build and verify the closed canonical autonomy authority/receipt subject and
  its domain-separated digests from typed scalar inputs. Orchestrator supplies
  a recommendation and already-issued authority evidence; it does not own the
  receipt bytes or digest algorithm. A Store supplies untrusted scalar rows and
  consumes the same verifier; it never reclassifies or rehashes the subject.
  For `PROCEED`, the Writer head commitment includes exactly the 15 scalar
  values that Store's fixed current-authority predicate proves in the same
  transaction; unasserted structural receipt fields are not durable authority.
- Classify a verified task-control stream as historical optional,
  required-receipt pending, or required-receipt complete. The one-event
  required prefix may exist only as a non-runtime-admissible reconciliation
  state; transition, completed replay, and Status require the exact receipt.

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
19. `AutonomyReceiptRecorded` is a closed typed event with only the frozen
    TASK-050 subject and authority-digest scalars; it cannot carry arbitrary
    JSON, commands, SQL, paths, credentials, prompts, or provider output.
20. Existing Task-created action families outside the bounded task-control
    namespace and the exact historical controlled-canary action synthesize no
    receipt. Their events, commands, receipts, projections, checkpoints, and
    hash domains replay byte-identically; an already present valid V1 receipt
    remains replayable.
21. `CONTROLLED_CODEX_CANARY_AUTONOMY_V1` is the only current required marker.
    It contains exactly one autonomy receipt event immediately after
    `TASK_CREATED` and before any later event or external effect. Other values
    in the reserved `CONTROLLED_CODEX_CANARY*` namespace, duplicate,
    missing-before-progress, late, reordered, or substituted receipts fail
    closed. Other action families are `NotApplicable`, not unknown profiles.
22. The event subject, event, terminal command receipt, stream head/resource
    projection/checkpoint, optional outbox admission, and physical Store
    receipt are one indivisible plan. A persistence adapter commits all or none.
23. An autonomy receipt records already-issued authority evidence; it does not
    grant, refresh, broaden, or execute authority and cannot authorize its own
    creation.
24. The `writer_lease_head_digest` is a commitment to the exact 15-scalar
    Writer current-authority assertion tuple. Caller-supplied projection fields
    outside that predicate cannot alter or enlarge the durable authority claim.
25. Public MCP tools, input schemas, and six-field output remain unchanged by
    the internal event. Projection-only status may derive from verified state
    but cannot become a second durable record or wire authority.
26. The event-kind contract cannot expose a value that the active PostgreSQL
    schema cannot persist. Schema-v5 readers continue to reject the withdrawn
    `INGRESS_RECEIPT_HANDOFF` spelling as unknown.

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

The in-memory fake is not restart evidence. Task Ledger 2.3 provides the pure
planner/checkpoint/replay boundary used by PostgreSQL, but does not itself
claim durable currentness. Postgres Store 1.10 must atomically commit the
command receipt, optional event, head, projection, outbox admission, checkpoint,
and physical Store receipt under daemon epoch/runtime-admission checks. Unknown
commit retries the same command/request and must reconstruct the identical
retained plan/receipt.

Version 2.2 preserves every 2.1 hash domain and historical byte. It adds only
the closed autonomy-receipt event/subject contract and mixed-profile replay.
The event is stored only through the schema-v5 Postgres Store adapter at
`0006`; databases with autonomy content at ordinal `0005` are incompatible and
must fail closed before migration DDL without repair.

Version 2.3 preserves the event, command, receipt, head, checkpoint,
`AUTONOMY_RECEIPT_RECORDED`, and migration `0006` bytes. It assigns the
Task-created profile discriminator and canonical autonomy subject/hash
semantics solely to Task Ledger. The discriminator is already covered by the
existing command-request/event/head/checkpoint hashes; it creates no second
profile hash or database column. Historical optional streams remain valid,
while the new required marker cannot progress or project Status without its
exact second event.

Version 2.4 proposed the closed `INGRESS_RECEIPT_HANDOFF` event for a completed
historical stream. It was accepted in ADR-024 but superseded before deployment
because schema v5 could not persist it and it did not cover the production
non-success terminal.

Version 2.5 withdraws that source-only event under ADR-025. No migration or
persisted event is rewritten: schema v5, existing event bytes, receipts, heads,
checkpoints, hash domains, Memory v3, and Writer Lease v2 remain unchanged.

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
| Autonomy event contract | fixed subject/authority fields, exactly-one ordering, full substitution/duplicate/missing/late matrices, and zero arbitrary payload surface | Security review | yes |
| Mixed historical replay | pre-autonomy streams replay byte-identically without a synthesized event; autonomy-enabled streams require the closed event and unchanged public MCP bytes | Compatibility review | yes |
| Autonomy atomicity | schema-v5 command, optional event, projection/checkpoint, terminal receipt, and physical receipt persist all-or-none | Engineering | yes |
| Schema/event parity | schema-v5 rejects the withdrawn handoff spelling and no unpersistable event remains in the public enum | Compatibility review | yes |
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
| 2.2 | 2026-08-14 | SPEC-002 v32, ADR-011/019, TASK-050, TASK-075 | Closed autonomy-receipt event/subject semantics, exactly-one ordering, and byte-identical mixed historical replay without public MCP or I/O expansion | User-approved TASK-075 reconciliation |
| 2.3 | 2026-08-15 | SPEC-002 v35, ADR-011/019, TASK-050 | Own the exact Task-created autonomy profile discriminator and canonical receipt verifier; preserve unrelated/historical action bytes while required profiles fail closed before progress or Status | User-approved TASK-050 repair amendment |
| 2.4 | 2026-08-24 | ADR-024, SPEC-008 v1 | Proposed a closed append-only successor ingress receipt handoff; superseded before deployment because schema v5 could not persist it | User-selected versioned handoff |
| 2.5 | 2026-08-24 | ADR-025, SPEC-008 v2 | Withdraw the unpersistable source-only handoff event while preserving all deployed schema-v5 and historical bytes | User-authorized bounded repair |
