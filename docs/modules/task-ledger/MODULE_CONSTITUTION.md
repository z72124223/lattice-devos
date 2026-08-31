---
module_id: task-ledger
name: Task Ledger
version: 3.3
status: active
owner: LATTICE maintainers
last_reviewed: 2026-08-30
---

## Mission

Own the versioned event, hash-chain, exact command-receipt, verified replay,
resource-projection, effect-intent outbox-admission, pure append-plan, and
complete checkpoint semantics, including the closed autonomy-receipt event,
the fixed foreman stream/event append plan and child-row verifier, and the
authoritative Task-created autonomy profile discriminator. Version 3.0 also
owns the distinct pre-specification general-task subject, canonical submission
envelope, task reference, and idempotency binding that PostgreSQL persists
beside the task stream as the single durable control-plane truth, plus the pure
intake-to-TaskSpec successor linkage and typed worker-attempt, exact lifecycle
observation, and independent-verification child-record semantics approved by
SPEC-011 and ADR-028. Version 3.1 fixes the exact observation-order contract
and adds the sole typed failed terminal permitted after exact thread/turn
acceptance but before `turn/started`. Version 3.2 adds the distinct typed
no-provider-effect retry predecessor approved by SPEC-011 and ADR-028. It
does not relabel that proof as a provider terminal or completion authority.
Version 3.3 also clarifies retained pre-v7 compatibility: a client key that was
historically valid in more than one stream has no canonical winner. Every
stream remains immutable, and a v7 lookup or mutation for that ambiguous key
is command substitution until separately reconciled by a future governed
operation.

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
- The separate create-only `GENERAL_TASK_INTAKE_V1` Task-created marker. It is
  valid only for a `GENERAL_TASK_INTAKE` stream, has no autonomy receipt, and
  permits no event after its single `TASK_CREATED` append.
- The versioned `TaskSubmissionEnvelope` for one create-only general task,
  including the process-owned ingress, caller idempotency key, exact objective,
  registered project display name, formal Project Registry authority-receipt
  digest, complete stream identity, derived stream ID, durable `task_ref`, and
  envelope digest. The objective is task data, never executable authority.
- The fixed `FOREMAN_COORDINATION` stream identity and versioned
  `FOREMAN_SNAPSHOT_RECORDED` event, including payload digest, exact-next generation
  order, exact command retry, and typed child-row replay verification.
- The immutable general-intake-to-TaskSpec successor binding committed by one
  `EVIDENCE_RECORDED` event, including the public task reference, both stream
  and creation-event digests, Project Registry receipt, TaskSpec, approval
  subject, budget, verification policy, and canonical binding digest.
- Typed worker-attempt rows committed by `EFFECT_INTENT`, exact provider
  thread/turn observation rows committed by `EVIDENCE_RECORDED` or
  `EFFECT_OUTCOME`, and independent-verification rows committed by
  `EVIDENCE_RECORDED`. These rows own evidence ordering and hash linkage only;
  they do not own Task Domain state, provider control, approval, Writer Lease,
  scheduling, or Artifact Store bytes.
- The closed `PRESTART_TERMINAL_FAILED` observation. It represents only an
  exact recovered failed terminal after thread and turn acceptance and before
  `turn/started`; it is terminal evidence but never an `EXECUTING` transition.
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
- Plan one fixed foreman append, exact retry, or changed-ID rejection against
  replay-verified event/command/child rows; verify exact-next generation and
  return the existing Ledger checkpoint without creating a current-state row.
- Construct and verify one canonical general-task submission envelope from
  bounded already-NFC inputs and a complete formal stream identity. Derive its
  public `task_ref` and envelope digest without accepting a path, command,
  credential, lease, approval, provider, or execution setting.
- Construct the matching `TASK_CREATED` append only with action
  `GENERAL_TASK_INTAKE_V1`, reason
  `GENERAL_TASK_INTAKE_RECORDED`, no diagnostic payload, and the envelope digest
  as its subject. Replay requires exactly that one event and no autonomy,
  transition, result, resource, outbox, or other effect record.
- Plan and replay one exactly-once intake-to-TaskSpec successor binding without
  appending executable work to the create-only intake stream.
- Plan and replay monotonic worker attempts, exact thread/turn observations,
  and independent verification child rows against their matching Ledger event,
  command, request, expected head, and canonical payload digest.
- Enforce the exact observation order `THREAD_ACCEPTED -> TURN_ACCEPTED ->
  TURN_STARTED -> ... -> TERMINAL`, with the sole pre-start exception
  `THREAD_ACCEPTED -> TURN_ACCEPTED -> PRESTART_TERMINAL_FAILED`. Completed or
  interrupted pre-start terminals and every pre-start `EXECUTING` projection
  are invalid.
- Export every managed-task record as complete typed scalars plus canonical
  payload bytes/digest, and reconstruct an explicitly untrusted row without
  reflection or accepting arbitrary authoritative JSON.

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
21. The current autonomy-receipt-required markers are exactly
    `CONTROLLED_CODEX_CANARY_AUTONOMY_V1` and `MANAGED_GENERAL_TASK_V1`. Each
    contains exactly one autonomy receipt event immediately after
    `TASK_CREATED` and before any later event or external effect. Historical
    controlled-canary and `GENERAL_TASK_INTAKE_V1` are not applicable. Other
    reserved values, duplicate, missing-before-progress, late, reordered, or
    substituted receipts fail closed.
22. A `GENERAL_TASK_INTAKE` identity contains a non-zero neutral intake digest
    and no Task Spec digest or accounting currency. It accepts only one
    `GENERAL_TASK_INTAKE_V1` `TASK_CREATED` event; autonomy state is exactly
    `NotApplicable`, and any second event or append attempt fails closed.
23. The event subject, event, terminal command receipt, stream head/resource
    projection/checkpoint, optional outbox admission, and physical Store
    receipt are one indivisible plan. A persistence adapter commits all or none.
24. An autonomy receipt records already-issued authority evidence; it does not
    grant, refresh, broaden, or execute authority and cannot authorize its own
    creation.
25. The `writer_lease_head_digest` is a commitment to the exact 15-scalar
    Writer current-authority assertion tuple. Caller-supplied projection fields
    outside that predicate cannot alter or enlarge the durable authority claim.
26. The internal event alone never widens MCP. ADR-027/SPEC-009 separately
    authorize one closed `lattice_foreman_checkpoint` adapter and a verified
    Runtime Status projection; neither becomes a second durable record or wire
    authority, and the legacy observer remains unchanged.
27. The event-kind contract cannot expose a value that the active PostgreSQL
    schema cannot persist. Schema-v5 readers continue to reject the withdrawn
    `INGRESS_RECEIPT_HANDOFF` spelling as unknown.
28. Every foreman child record binds one matching Ledger event, command,
    request digest, payload digest and generation. Missing, duplicate, changed,
    reordered, or cross-stream records fail replay; exact retry is byte-equal.
29. A general-task envelope binds one exact objective and formal project
    authority receipt to one complete Task Ledger identity. Its `task_ref` and
    envelope digest change if any authoritative input changes.
30. Objective and project-display-name text must be non-empty, trimmed,
    already NFC, bounded, free of NUL/control characters, and free of
    recognized secret material. Debug and error projections redact the human
    text; diagnostic JSON never stores a second copy.
31. Task-ingress idempotency is the exact pair of process-owned `ingress_id`
    and `client_request_id`, shared across controlled-canary and general intake.
    The key must satisfy the `lattice-contracts` one-to-64-byte secret-free
    ASCII predicate before project resolution and again at durable admission.
    An exact envelope retry returns the retained task; changed objective,
    project binding, or submission mode is substitution and discloses no
    different task. Task Ledger owns this meaning even though Postgres Store
    owns the unique index and transaction mechanics.
32. The submission envelope is an authoritative intake binding, not lifecycle
    state, an execution-ready Task Spec, a Policy decision, an approval, or a
    writer lease. Persisting it grants no model, process, filesystem, Git,
    payment, external-action, merge, deployment, or release authority.
33. One verified create-only intake links to at most one TaskSpec successor for
    the same Project Registry snapshot and Task ID. Exact command retry is
    byte-equal; changed task, stream, TaskSpec, approval subject, budget,
    verification policy, or linkage is substitution.
34. The successor binding is committed by exactly one matching
    `EVIDENCE_RECORDED` event. Missing, duplicate, unknown-schema, reordered,
    cross-stream, or digest-changed binding rows fail replay.
35. Worker attempt numbers begin at one and advance by exactly one. Writer
    fences strictly increase and attempt IDs never repeat. Attempt N+1 is
    invalid until attempt N has either exact terminal observation evidence or
    one owner-verified no-provider-effect predecessor that binds the same task,
    prior attempt and fence, immutable original blocker, distinct
    reconciliation-proof digest, and exact successor packet digest. Missing,
    foreign, digest-colliding, substituted, or replay-changed predecessor
    evidence fails closed. The no-provider-effect predecessor is not a Codex
    terminal and cannot authorize verification, completion, Writer release,
    merge, deployment, or publication.
36. The first observation binds one provider thread; a later observation may
    add one turn, but neither identifier may ever change for that attempt.
37. A worker-attempt row binds one `EFFECT_INTENT`; a nonterminal observation
    binds `EVIDENCE_RECORDED`; a terminal observation binds `EFFECT_OUTCOME`;
    and a verification binds `EVIDENCE_RECORDED`. The child payload digest is
    the exact event subject digest.
38. Verification rows require a prior exact terminal for the same attempt and
    commit only closed profile, Git/evidence/result/review digests. They never
    accept a command, path, prompt, raw provider output, or artifact bytes.
39. Managed-task child rows are evidence beneath the one successor Task Domain
    stream. Their lifecycle labels cannot transition, authorize, complete, or
    create a second task state machine.
40. `THREAD_ACCEPTED` alone binds no turn and `TURN_ACCEPTED` does not prove
    execution. `TURN_STARTED` is the only observation that opens the normal
    execution/terminal sequence. `PRESTART_TERMINAL_FAILED` is accepted only
    after the exact accepted turn, carries that same thread/turn, is always
    `FAILED`, and closes without ever synthesizing `TURN_STARTED`.
41. The v7 unique ingress key governs all new claims but cannot retroactively
    erase, merge, rename, or choose between distinct pre-v7 streams that
    retained the same valid command suffix. Such a key is an explicit
    historical ambiguity: no active claim exists, all exact stream/event/
    command identities remain durable, and read/prepare/record return command
    substitution without disclosing a different task.

## Allowed Dependencies

- Rust standard library.
- `lattice-contracts` 1.15 immutable shared values and Task Ledger receipt/head
  representations, whose Ledger-specific semantics remain unchanged from 1.3.
- `lattice-cjson` 1.0 canonical-byte/hash mechanics only.
- `lattice-foreman-state` 1.5 only for the closed snapshot/checkpoint input,
  replay-projection semantics, worker-model allowlist, and reasoning-effort
  values owned by that module.
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

Version 2.6 adds the reserved foreman stream/event and typed payload commitment
without changing historical event bytes. PostgreSQL may persist a fixed-scalar
child only beside the matching Ledger append in one transaction; diagnostic
JSON, hypotheses, and child rows cannot become lifecycle authority.

Version 2.9 corrects the unreleased Phase 3 intake model. General intake is a
distinct stream subject with a neutral digest and no Task Spec digest,
accounting currency, autonomy receipt, transition, or result. Its exact
`GENERAL_TASK_INTAKE_V1` create-only marker replaces the initial erroneous
required-profile proposal. Existing canary and historical event/hash/profile
bytes remain unchanged. Postgres Store 1.22 may persist the shared ingress
claim and envelope only in the same transaction as the matching Ledger append
and must replay-verify all three before returning it.

Version 3.0 preserves all historical stream/event/head/checkpoint bytes and the
create-only intake rule. It adds only closed new action families and typed
child-record hash domains for the unique TaskSpec successor lineage, monotonic
worker attempts, exact provider observations, and independent verification.
The same-database `foreman-execution/v1` extension owns physical uniqueness,
atomicity, locks, and rows while Store-v7 and migrations 0001 through 0008 stay
unchanged; this module remains pure and rejects every missing or substituted
row during replay.

Version 3.1 preserves prior hash domains and physical row shapes while
tightening the semantic observation verifier. It adds the typed pre-start
failed-terminal spelling and corrects the required-autonomy profile list to
include the managed successor marker.

Version 3.2 preserves every prior event, terminal-observation, head, checkpoint,
and physical row meaning. It adds a separate typed retry-predecessor verifier
for the owner-atomic no-provider-effect closure already approved in SPEC-011
and ADR-028. Exact terminal evidence remains the ordinary retry predecessor;
closure evidence is accepted only for bounded attempt admission and never
changes terminal, verification, completion, or Writer-release semantics.
Version 3.3 changes no Task Ledger hash, stream, event, command, receipt, or
planner bytes. It closes only the interpretation of a production-retained
pre-v7 duplicate key: neither persistence migration nor runtime may invent a
winner. Postgres Store 1.23 may retain exact ambiguity lineage as physical
metadata, while Task Ledger continues to expose the existing static command-
substitution category for every attempted use of that key.

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
| General intake envelope | objective/project/authority/identity substitution, strict text/secret bounds, stable `task_ref`, Debug redaction, distinct subject kind, absent spec/currency, and no-autonomy matrices | Security review | yes |
| General intake persistence parity | shared canary/general ingress-key collision, exact retry, changed-key rejection, and pre-v7 multi-stream ambiguity use the same claim/envelope verifier; ambiguity preserves all exact lineage and exposes no winner; claim plus envelope plus one `TASK_CREATED` append commit all-or-none and replay across restart; Registry snapshots accept the closed 159-byte maximum and reject 160 bytes | Engineering | yes |
| Managed-task runtime child records | unique successor binding, exact retry/substitution, monotonic attempts/fences, exact-terminal or owner-verified no-provider-effect predecessor before retry, closure foreign/fence/digest/packet substitution rejection, immutable thread/turn, verification-after-terminal, canonical adapter export/import, and missing/duplicate/tamper replay matrices | Engineering and Security review | yes |
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
| 2.6 | 2026-08-25 | SPEC-006 v3, ADR-024/025, TASK-079/087/094 | Add fixed foreman stream/event generation, typed payload commitment and child-row replay verification without changing historical bytes | Fixed-foreman delegation and TASK-105 integration |
| 2.7 | 2026-08-25 | SPEC-009, ADR-027, TASK-105 | Require exact-next foreman generation and narrow the separately approved MCP adapter/status projection without changing Ledger bytes or legacy MCP | Sole-foreman delegation |
| 2.8 | 2026-08-26 | ADR-023 Phase 3 amendment | Initial general-intake envelope and required-profile model; superseded before release by 2.9 because intake must not fabricate Task-Spec/autonomy semantics | User-authorized Phase 3 |
| 2.9 | 2026-08-26 | ADR-023 Phase 3 P1 correction | Separate general intake from Task Spec, remove currency/autonomy/progression, and retain one create-only event in the shared ingress-idempotency namespace | User-authorized Phase 3 |
| 3.0 | 2026-08-26 | SPEC-011, ADR-028 | Add the pure exactly-once TaskSpec successor lineage and typed worker-attempt, exact lifecycle observation, and independent-verification child-record domains without adding I/O or a second task state machine | Delegated product owner |
| 3.1 | 2026-08-27 | SPEC-011, ADR-028 durable-core review | Lock exact observation order, add the sole typed failed pre-start terminal without entering Executing, and correct the managed autonomy-required marker contract | Delegated product owner |
| 3.2 | 2026-08-28 | SPEC-011 v1.7, ADR-028 retained pre-start amendment | Admit attempt N+1 from either the ordinary exact terminal or an owner/task/fence/proof/successor-packet-bound no-provider-effect predecessor, without treating closure as provider terminal or completion authority | User-authorized Phase 4 |
| 3.3 | 2026-08-30 | ADR-023 deployment compatibility amendment | Define pre-v7 multi-stream ingress keys as fail-closed ambiguities with no winner while preserving every Task Ledger identity and all existing semantic bytes | User-authorized deployment hotfix |
