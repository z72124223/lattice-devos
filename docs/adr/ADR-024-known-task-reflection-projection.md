# ADR-024: Known-Task Reflection Projection

- Status: accepted for GH-9 local implementation; merge/release remain
  unauthorized
- Date: 2026-08-21
- Decision owner: user GH-9 delegation
- Related: GH-9, SPEC-003 v5, ADR-011, ADR-019, ADR-023, Task Domain 2.3,
  Ports 1.9, latticed 1.5

## Context

After a Codex task reaches durable completion, LATTICE still needs a bounded
Reflection lane for Hermes failure learning and candidate recording. Reusing
the core Task lifecycle state for that lane would make Reflection tails appear
to rewrite successful core completion, while giving Hermes a lifecycle
repository or database client would violate One Gateway, One Truth, One Writer.

The first GH-9 slice is intentionally a known-Task lane. A bounded caller
already has the exact Task binding and explicitly invokes typed ports. There is
no cross-task discovery, no `claim_next`, no supervisor, no MCP expansion, and
no automatic Hermes execution.

## Decision

Task Domain 2.3 owns only closed Reflection vocabulary and pure projection
rules: `REFLECTION_PENDING`, `REFLECTION_FAILED`, `RETRY_PENDING`,
`DEGRADED`, failure kinds, and candidate kinds. These values do not add core
Task states and do not change Task Spec 2.1 hash subjects or transition
legality.

Ports 1.9 adds three abstract Reflection boundaries:

- `TaskReflectionQueuePort` for explicit known-Task pending, claim, failure,
  retry, degraded, and load operations;
- `HermesTaskReflectionHistoryPort` for bounded digest-only history pages;
- `HermesTaskReflectionCandidatePort` for candidate appends bound to the exact
  authorized history page digest.

Reflection events are immutable Task Ledger appends over the same verified
stream, but they are not `STATE_TRANSITION` events. Queue admission is an
`EFFECT_INTENT`; claims and candidates are `EVIDENCE_RECORDED`; failures,
retry-pending, and degraded receipts are `EFFECT_OUTCOME`.

The core replay and Reflection replay are separate projections:

- the core projection keeps the last legal core-event head and terminal result;
- the public journal head remains the head of the full append-only stream;
- Reflection appends cannot alter `TaskState::Completed`, the result digest,
  original core events, or the core-head digest.

Hermes-facing history is a typed keyset page. Its digest commits the exclusive
sequence cursor, limit, returned events, next cursor, exact core anchor, and
current journal head. Candidate append replays that same page and rejects stale
or substituted history before accepting the candidate digest.

Only completed core Tasks can enter the queue/claim/candidate/retry/degraded
lane. Direct `TASK_FAILURE` and fixed-verifier `OUTPUT_REJECTED` records on a
failed core are terminal read-only Reflection history; they cannot claim,
retry, degrade, append a candidate, or revive the core Task in this slice.

## Consequences

- LATTICE can preserve a completed core result while recording later Reflection
  failures and candidates.
- Hermes receives bounded digest-only history and a closed candidate append
  surface, not SQL, raw prompts, stderr, credentials, update/delete authority,
  or a core-transition operation.
- The existing PostgreSQL-backed Task Ledger can persist GH-9 without a new
  physical migration because the projection is derived from the verified
  append-only event stream.
- A future global scanner, `claim_next`, unattended worker, MCP tool, or
  automatic Hermes caller requires a separate versioned decision.

## Acceptance Evidence Required

- Unit tests for core-head immutability under Reflection tails.
- Unit tests for independent Reflection replay, bounded history, stale-page
  rejection, candidate binding, retry/degraded projection, and failed-core
  terminal history.
- Live marker-owned PostgreSQL replay proving `COMPLETED` core plus
  `REFLECTION_FAILED` projection survives a physical database restart in a
  fresh process.
- Governance trace through Task Domain 2.3, Ports 1.9, latticed 1.5, and
  SPEC-003 v5.

No PR, merge, default branch update, deployment, or release is authorized by
this ADR.
