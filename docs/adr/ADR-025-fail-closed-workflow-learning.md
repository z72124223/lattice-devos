# ADR-025: Fail-Closed Workflow Learning And Pre-Stop Challenge

- Status: accepted for the bounded TASK-084 characterization slice
- Date: 2026-08-21
- Decision owner: user through fixed foreman delegation
- Related: ADR-001, ADR-005, ADR-007, ADR-011, ADR-012, GH-9, TASK-048,
  TASK-049, TASK-077, TASK-078, TASK-079, TASK-080

## Context

The foreman inferred no work from quiet windows, idle/UNKNOWN projection state,
and sidebar/dashboard shape. The user challenged this. A full backlog audit
then found stale and contradictory evidence plus safe independent work. A
learning loop is needed, but allowing a model to edit executable policy or
authority would turn one coordination defect into a control-plane escalation.

## Decision

Workflow learning is a closed typed state machine. Observations produce
expiring epistemic records with multiple hypotheses, confidence/unknowns,
support and counterevidence, assumptions/scope, checked/expiry time, refresh
triggers, decision, expected/actual signal, falsifier, and a bounded reversible
next probe. The hard-coded behavior is the inquiry/safety process, never the
answer to a disputed incident. `UNKNOWN` remains unknown. Candidates contain
only allowlisted procedural changes; multi-case/counterexample replay and
machine receipts precede promotion; the previous active policy is retained;
monitored hard-limit degradation rolls back.

Every zero-work/all-waiting/healthy/end-round claim first runs a counterexample
challenge across inventory, tickets, PLANS, dashboard, Git/worktree,
dependencies, stale or failed tools, unopened backlog, decomposable blockers,
and safe low-cost GPT-5.6 work. A counterexample forbids stopping. Stopping is
possible only with `lattice.zero-work-certificate/1.0`, complete fresh source
coverage, and an exact non-ready reason for every candidate. Quiet is evidence
of nothing.

Protected authority and hard cost limits are outside candidate data. Payment,
credentials, public network, force push, default merge, deployment, release,
archive, and security-control changes cannot be represented as a promotable
rule. Unknown change kinds reject.

New counterevidence reduces confidence. Expiry, low confidence, changed
dependencies, or tool failure triggers refresh and prevents a stale conclusion
from remaining active. Every probe declares the hypotheses it distinguishes,
expected/falsifying signal, cost/risk limit, and stop condition; its result may
overturn the prior decision. No single case can promote policy.

Normal operation maintains an expiring suspicion queue across product,
integration, dependency, Git/worktree topology, truth-source, resource/cost,
security/authority, and user-value domains. Priority favors valuable,
consequential, fresh, cheap, reversible, non-conflicting probes. A suspicion is
not a defect. Fresh duplicates, repeated validation, manufactured work, and
governance-only busywork are rejected or deprioritized.

TASK-083 supplies the first path-topology case: a nested worktree polluted its
parent projection. The safe probe compares resolved/approved roots, nesting,
reparse escape, branch/HEAD and parent cleanliness; the only repair candidate
is verified Git-native `worktree move`. The successful actual outcome is
recorded, while promotion of a general path invariant still needs a distinct
counterexample case.

Task Ledger remains the future semantic durable owner, Postgres Store the
physical owner, and Writer Lease the mutation authority. TASK-084 consumes a
future TASK-079 snapshot/replay contract but does not copy its uncommitted code.

## Consequences

- The current slice can independently prove replay, challenge, promotion,
  monitoring, rollback, cost, and authority behavior with pure tests.
- Production promotion and durable fresh-process proof remain blocked on a
  committed TASK-079 integration contract.
- Dashboard, tickets, PLANS, Git, chat, and model outputs remain evidence or
  projections, never a new truth or writer.

## Rejected Alternatives

- Free-form model-authored rules: unreviewable executable authority.
- Dashboard-backed learning state: projection becomes second truth.
- Silence/idle thresholds as completion: repeats the incident.
- Promotion from test counts alone: no binding to policy version, fence,
  evidence, safety regression, or cost.
- Encoding the TASK-080 answer as a permanent rule: confuses one incident with
  durable knowledge and prevents future counterevidence from correcting it.
- Updating the active version in place: removes rollback and replay identity.
