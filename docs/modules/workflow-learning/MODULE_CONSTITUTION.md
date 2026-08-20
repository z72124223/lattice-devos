---
module_id: workflow-learning
name: Workflow Learning
version: 1.0
status: proposed
owner: LATTICE maintainers
last_reviewed: 2026-08-21
---

## Mission

Own the closed, versioned, fail-closed semantics that turn evidenced foreman
workflow mistakes into replay-tested policy candidates, machine-gated
promotion, monitored activation, and deterministic rollback.

## Non-Goals

- Execute workers, schedule tasks, mutate Git/dashboard/PostgreSQL, call a
  model, acquire a Writer Lease, approve work, or archive a task.
- Accept free-form rule code, self-approval, raw chats, prompts, commands,
  environments, provider output, secrets, credentials, or arbitrary paths.
- Change protected authority, safety, dependency, resource, cost, merge,
  deployment, or release boundaries.

## Owned Data

- Closed observations, multiple hypotheses, confidence/unknowns, supporting and
  counterevidence, assumptions/scope, freshness/expiry/refresh triggers,
  decisions, expected/actual signals, falsifiers, bounded probes,
  deduplication identity, policy candidates, versions, fences, machine gates,
  pre-stop counterexamples, zero-work certificates, monitor metrics, and
  rollback decisions.
- Expiring suspicion queue entries, cross-domain probe priority, and pure
  worktree-topology classifications/repair receipts.

Task Ledger will own durable append/replay after TASK-079; Postgres Store will
own physical rows; Writer Lease will own mutation authority. The dashboard,
PLANS, Git, tickets, chats, models, and this module's in-memory checkpoint are
not independent durable truth.

## Public Contracts

- Observe bounded secret-free evidence pointers idempotently.
- Preserve multiple reasonable hypotheses and degrade conflicting, expired, or
  low-confidence records to unknown/refresh.
- Run bounded reversible probes that declare hypotheses, cost/risk ceiling,
  expected signal, falsifier, and stop condition before execution.
- Build only closed policy candidates against an exact base version/fence and
  hard cost limit.
- Replay historical/fault snapshots and challenge every pre-stop claim.
- Issue a zero-work certificate only for a fresh exhaustive inventory with one
  exact non-ready reason per candidate.
- Rank ongoing refutable hypotheses by value/risk/freshness/probe cost,
  reversibility, and conflict without labeling them defects.
- Classify resolved/approved/nested/reparse/parent-clean worktree evidence and
  permit only a verified Git-native worktree move repair receipt.
- Promote only after the complete exact machine-gate set passes; retain the
  previous policy and auto-rollback on hard-limit degradation.

## Invariants

1. Unknown, idle, quiet, sidebar count, and projection success never prove no work.
2. A counterexample makes stop/certificate issuance impossible and requests reconciliation.
3. Untrusted, stale, duplicated, malformed, or single-source reports cannot support promotion.
4. Policy version and fencing are strictly monotonic; stale base/fence rejects.
5. The previous active policy remains replayable until the new policy survives monitoring.
6. Protected authority and hard cost ceilings are immutable and cannot appear as learnable fields.
7. Real dependency, resource, duplicate-work, and safety conflicts remain dispatch denials.
8. Every decision is pure and side-effect-free in this module.
9. One incident can never promote policy; multiple cases, counterexamples,
   cost/safety regression, and rollback evidence are mandatory.
10. Hard rules govern inquiry and safety gates, never the answer to an open
    hypothesis; `UNKNOWN` remains unknown until fresh evidence distinguishes it.
11. Fresh duplicate suspicions, manufactured failures, repeated proof, and
    governance-only busywork cannot consume priority over higher-value product probes.
12. Nested or escaped worktree topology prevents healthy/completion/zero-work
    certification; manual delete/move, reset, clean, and blind recreation are
    never recommended.

## Allowed Dependencies

- Rust standard library.
- Future versioned read-only foreman-state identifiers after TASK-079 is committed.
- Future `lattice-cjson` canonical digest mechanics only.

## Forbidden Dependencies

- Task Ledger, Postgres Store, Writer Lease, Orchestrator, dashboard exporter,
  Git/filesystem/process/network/database clients, provider SDKs, credentials,
  product repositories, or executable policy engines.

## Failure, Compatibility, And Migration

Unknown schemas, codes, versions, gates, evidence sources, or monitor metrics
reject without changing active policy. Schema changes are additive by new
version; old observations/policies remain replayable. Durable integration must
be atomic under Task Ledger plus Writer Lease fencing and must preserve the
previous policy for rollback.

## Acceptance Gates

| Gate | Evidence | Owner | Required for merge |
|---|---|---|---|
| Incident/pre-stop replay | TASK-080 and zero-work fixtures plus fault tests | Engineering | yes |
| Learning integrity | trust, malicious-input, dedup, freshness tests | Security review | yes |
| Epistemic lifecycle | multiple hypotheses, confidence degradation, expiry, refresh and falsifying probe tests | Engineering | yes |
| Promotion/fencing | complete gate, version, fence, no-evidence tests | Engineering | yes |
| Protected authority/cost | immutable authority and hard-cap rejection matrix | Security review | yes |
| Recovery/rollback | fresh checkpoint and regression monitor tests | Engineering | yes |
| Suspicion queue | ranking, expiry, duplicate/refutation and product-value preservation | Product/architecture review | yes |
| Worktree topology | TASK-083 actual outcome plus nested/reparse/cross-case tests | Security review | yes |
| Durable binding | TASK-079 Ledger/Writer Lease/Postgres conformance | Architecture review | yes |

## Change Policy

Observation meaning, diagnosis classes, evidence trust, candidate schema,
machine-gate set, certificate rules, policy/fence ordering, cost/protected
authority boundaries, or rollback behavior require a versioned amendment,
threat and architecture review, and responsible-user authorization.

## Amendment History

| Version | Date | Decision reference | Summary | Approver |
|---|---|---|---|---|
| 1.0 | 2026-08-21 | ADR-025, SPEC-007, TASK-084 | Initial fail-closed learning and pre-stop challenge boundary | Fixed foreman delegation |
