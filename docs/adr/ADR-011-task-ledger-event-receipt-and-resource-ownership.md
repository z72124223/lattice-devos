# ADR-011: Task Ledger Event, Receipt, Replay, And Resource Ownership

- Status: accepted for TASK-013 under the user's 2026-07-29 directive to
  continue the approved LATTICE plan through MVP-3
- Date: 2026-07-29
- Decision owner: user
- Related: SPEC-002 v35, ADR-001, ADR-005, ADR-008, ADR-009, ADR-019,
  TASK-013, TASK-050, TASK-075

## Context

V1 demonstrates useful hash-chain, replay, idempotency, and sanitization
intent, but implements them with per-task files, Node canonical JSON, an
in-process queue, heuristic arbitrary-payload redaction, and a direct Task
Ledger to Task Domain dependency. Those mechanisms cannot be the V2 durable
authority.

Policy 2.3 also represents Task-Ledger-owned resource evidence as
caller-constructible strings plus a `fresh` Boolean. That repeats the
owner/currentness weakness corrected for Project Registry in TASK-012.

## Decision

Create a pure Rust `lattice-task-ledger` 2.0 module. It owns:

- task-stream identity and the zero/non-zero stream-head rules;
- versioned command-request, event, stream-head, command-receipt, resource
  projection, resource-observation, and resource-receipt hash subjects;
- exact `(stream_id, command_id)` retry semantics;
- immutable verified event/receipt representations plus explicitly untrusted
  persistence records and pure verified replay;
- corruption, unknown-version/event, and claimed-head/projection mismatch
  failure semantics;
- Ledger-owned resource-counter projection and observation issuance.

The TASK-013 fake stores only process-memory characterization state. It is not
restart evidence, PostgreSQL truth, a live authority, or permission to execute
an effect.

## Stream Identity And Head

A stream is not identified by a naked Task ID. Its identity binds:

- canonical Project ID;
- immutable Project Registry snapshot ID;
- validated Task ID;
- canonical positive Task Spec revision;
- Task Spec SHA-256;
- one canonical accounting currency;
- stream kind `TASK` for version 2.0.

The stream ID is a domain-separated digest of that complete identity. Version
2.0 does not admit a caller-defined guardian/system-stream Boolean or arbitrary
stream kind.

Every full head binds the fixed producer/version/runtime, complete stream
identity, stream ID, sequence, last event digest, resource revision, resource
projection digest, and head digest. Sequence zero requires the zero event
digest and zero resource projection; a non-zero sequence requires a non-zero
event digest. Append supplies the complete expected head, not only a sequence.

## Event Contract

Version 2.0 events use a closed `LedgerEventKind`. Every event binds:

- schema version and complete stream identity;
- exact sequence and predecessor digest;
- command/request/correlation identity;
- caller-supplied canonical UTC RFC 3339 timestamp;
- closed event kind and outcome;
- validated actor/action/reason identifiers;
- an authoritative subject digest;
- an optional bounded sanitized diagnostic;
- an optional typed resource snapshot only for the resource-snapshot event;
- the resulting resource projection revision/digest;
- the event digest.

No event ID or timestamp is allocated after retry lookup. Event identity is the
stream ID, sequence, and event digest. Task-state legality is not duplicated in
this crate.

Authoritative event fields are typed and never inferred from diagnostic text.
The diagnostic surface is optional, bounded, non-authoritative, NFC, and
sanitized before request/event hashing. Known secret-bearing forms are
redacted; NUL, oversize, or still-recognizable secret forms reject. Two
diagnostics that reduce to the same sanitized text intentionally have the same
diagnostic semantics. Callers must bind distinct authoritative operations with
distinct subject digests, not hidden diagnostic text.

## Hash Domains

`lattice-cjson-1` supplies mechanics only. Task Ledger owns at least these
separate version-2.0 schema domains:

- `lattice.task-ledger.stream-id`;
- `lattice.task-ledger.command-request`;
- `lattice.task-ledger.event`;
- `lattice.task-ledger.stream-head`;
- `lattice.task-ledger.command-receipt`;
- `lattice.task-ledger.resource-projection`;
- `lattice.task-ledger.resource-observation`;
- `lattice.task-ledger.resource-receipt`.

All semantic identifiers must already be NFC and satisfy their closed ASCII or
canonical scalar contract before map lookup or hashing. PostgreSQL `JSONB`
output is never hash input.

## Command And Receipt Semantics

The idempotency key is exactly `(stream_id, command_id)`.

1. Validate and sanitize the complete request, then compute its request digest.
2. Look up the command key before expected-head evaluation.
3. Same key and same request digest returns the byte-identical terminal
   receipt, even when later commands advanced the stream.
4. Same key and another request digest returns `COMMAND_ID_REUSE`.
5. The same command ID in another stream is independent.
6. A new stale/mismatched head or sequence overflow creates a stable terminal
   denied receipt but appends no event and changes no stream/resource head.
7. An appended receipt binds request, before/after full heads, outcome/reason,
   event digest, and receipt digest.

Receipt replay metadata is not stored inside the receipt, so first execution
and retry compare equal. The fake builds the event, resulting projection/head,
and terminal receipt completely before mutating its maps.

The durable command row must retain the complete canonical request source in
addition to its request digest and terminal receipt. A denied command has no
event from which to reconstruct that request, so a digest-only row cannot prove
request/receipt binding during replay. Raw persistence records are explicitly
untrusted, preserve unknown schema/kind/outcome text for fail-closed parsing,
and elide all raw fields from `Debug`.

The future PostgreSQL implementation must preserve this lookup order and
terminal receipt meaning inside the ADR-005 transaction. Unknown database
commit status retries with the same command and request digest.

## Replay And Projection Boundary

Task Ledger verifies schema/event kind, stream binding, exact sequence,
predecessor/event hashes, command/request binding, receipt correspondence,
resource projection, and claimed full head. Missing, duplicate, reordered,
truncated, substituted, orphaned, or unknown records fail closed.

The public replay boundary consumes one complete untrusted stream snapshot:
identity, claimed full head/projection, raw events, and raw command-key/request/
receipt rows. It reconstructs typed commands and verified events, verifies
every appended and denied receipt, rejects duplicate/cross-stream keys and
extra appended receipts, and returns only a typed verified stream. The fake
exports through and delegates to this same pure boundary; no private-map-only
verification path exists.

Task Ledger owns only chain replay and its resource projection. Task Domain
continues to own legal task states/transitions. Future Orchestrator composition
passes a verified event view to a separately versioned Task Domain reducer and
compares the persisted rebuildable Task Packet projection. Therefore:

```text
lattice-task-ledger -> lattice-contracts + lattice-cjson
lattice-task-ledger -X-> lattice-task-domain
lattice-task-domain -X-> lattice-task-ledger
future lattice-orchestrator -> both public contracts
```

Policy's normal/production dependency graph remains the graph above. A
TASK-013-only `dev-dependency` from Policy tests to Task Ledger obtains an
actual fake-owner current head for cross-crate composition evidence; it is not
compiled into the Policy library and does not transfer event, receipt, or
resource ownership.

## Resource Observation Boundary

`lattice-contracts` 1.3 provides neutral immutable representations for:

- the full Task Ledger stream head;
- checked current/requested resource usage;
- a fixed-producer/version/runtime resource observation receipt;
- a full resource observation head mirroring every security field.

The receipt binds complete task/project/spec identity, current full stream
head, resource revision/projection, exact effect claim, one currency, current
and requested counters, observation digest, and receipt digest.

Policy 2.4 accepts only a receipt whose full projection equals a head obtained
from an independent current Task Ledger owner lookup and whose exact claim
matches the decision subject. It has no caller-owned `fresh` Boolean or
caller-selected producer/owner strings.

Like the Registry boundary, a receipt's own `head()` is structural projection,
not currentness or authenticity proof. TASK-013 tests currentness through the
fake owner's `current_resource_head` lookup and passes that result into Policy.
Future Orchestrator/PostgreSQL must authenticate and serialize that lookup.
Before a real side effect, PostgreSQL must re-check and claim the resource
counters, daemon epoch, runtime admission, and effect/outbox intent in one
transaction. A read observation alone is susceptible to time-of-check to
time-of-use drift and never authorizes a live effect.

## Compatibility

- V1 file readers and candidate hashes remain read-only characterization.
- No V1 hash is promoted into the approved compatibility manifest by this ADR.
- V1 unknown-event no-op, arbitrary payload, Task Domain import, file
  persistence, and heuristic sanitizer behavior are explicitly rejected.
- Changing a field set, hash domain, event kind, counter meaning, receipt
  outcome, or retry order requires a versioned amendment and fixtures.

### TASK-075 schema-v5 autonomy amendment

Task Ledger 2.3 owns the closed `AutonomyReceiptRecorded` event defined by
TASK-050. The event owns fixed scalar fields for its canonical autonomy receipt
subject and authority digest; it does not accept arbitrary JSON, paths,
commands, SQL, credentials, or provider payloads. Planning and verified replay
remain pure Task Ledger semantics. Durable storage belongs only to Postgres
Store and must append the command, optional autonomy event, stream projection,
terminal domain receipt, and physical persistence receipt atomically.

The global migration order is Registry `0005` / schema v4 followed by autonomy
`0006` / schema v5. Existing Task Ledger events, commands, receipts,
checkpoints, and Store-v2 receipts remain byte-identical. Historical Registry
persistence receipts use their command-owned schema/manifest profile rather
than the current global profile. No public MCP field or tool changes, and no
autonomous scheduler, model call, Git effect, provider effect, or authority
expansion, are part of this amendment.

### TASK-050 profile and receipt-owner repair amendment

Task Ledger 2.3 owns the closed
`lattice.task-ledger.task-created-profile/1.0` discriminator for bounded
task-control streams. Its carrier is the existing authoritative
`TASK_CREATED.action`, already covered by the command-request, event, head, and
checkpoint hashes; no new event field, standalone hash, or database column is
introduced.

The closed task-control mappings are:

- `CONTROLLED_CODEX_CANARY` -> historical autonomy-optional V1;
- `CONTROLLED_CODEX_CANARY_AUTONOMY_V1` -> autonomy-receipt-required V1.

Other existing Task-created action families are outside this discriminator and
retain byte-identical historical semantics. Other values in the reserved
`CONTROLLED_CODEX_CANARY*` namespace fail closed as unknown profiles. The
required profile with only `TASK_CREATED` is a reconciliation-only pending
prefix: its next event must be the exact sequence-2
`AUTONOMY_RECEIPT_RECORDED`. It cannot transition, replay as completed, or
project normal Status until that event verifies. Historical optional streams
may omit the event; if already present, it remains unique at sequence 2.

Task Ledger alone constructs, canonicalizes, hashes, orders, and verifies the
closed authority/receipt subject. The generic append API cannot accept a
caller-forged autonomy subject digest. Orchestrator retains only its pure
recommendation; Ports transports typed verified values; Postgres Store maps
fixed scalars and delegates untrusted-row verification back to Task Ledger.

## Consequences

- The semantic owner is ready before the physical PostgreSQL adapter.
- Contracts and Policy receive versioned resource-receipt hardening without a
  Ledger/Policy dependency cycle.
- Full AC-03 PostgreSQL atomicity and durable/restart portions of AC-04 remain
  open.
- The fake proves deterministic semantics only and must report
  `RuntimeKind::Fake`.

## Verification

- exact retry after later append and full one-field request mutation matrix;
- cross-stream command scoping and stale sequence/hash/stream/overflow denial;
- domain, Unicode, key-order, null/missing, and unknown-version/event fixtures;
- tamper, reorder, truncate, duplicate, orphan-receipt, head, and projection
  mismatch rejection;
- bounded diagnostic redaction/rejection with no raw-secret Debug/error leak;
- resource producer/runtime/project/task/spec/head/revision/claim/currency/
  counter substitution and historical-head denial;
- atomic no-partial-mutation checks for every failure;
- focused/full format, lint, Rust/Node, dependency, and forbidden-I/O checks.
