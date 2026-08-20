---
spec_id: SPEC-007
title: Fail-closed workflow learning and strengthening loop
status: approved_for_bounded_characterization
version: 1
approved_by: fixed_foreman_delegation
approved_at_local: 2026-08-21
modules:
  - module_id: workflow-learning
    constitution_version: 1.0
---

# SPEC-007 - Fail-closed workflow learning and strengthening loop

## Problem

The fixed foreman treated a quiet sidebar, idle windows, and a partial dashboard
with many `UNKNOWN` items as if they proved a large project had no dispatchable
work. User challenge and TASK-080's full audit found stale projections,
ticket/PLANS/dashboard/Git contradictions, unexamined backlog, and several safe
parallel tasks. Silence and projection status were incorrectly promoted into a
stop decision.

## Intended Behavior

The module implements one closed cycle:

1. **OBSERVE** records a bounded incident code and an expiring epistemic record:
   multiple live hypotheses, confidence and unknowns, supporting evidence
   pointers, counterevidence, assumptions/scope, `checkedAt`, `expiresAt` and
   refresh triggers, current decision, expected/actual signal, falsifier, and
   the next reversible probe. It never stores a complete chat, prompt, command,
   environment, secret, credential, or raw provider output.
2. **DIAGNOSE** keeps multiple reasonable hypotheses alive. Stale dashboard,
   missing/contradictory ticket, real dependency blocker, resource conflict,
   duplicate work, released dependency, unopened backlog, tool/path coverage
   failure, and low-cost GPT-5.6-safe work are hypotheses to distinguish, not
   conclusions to hard-code. `UNKNOWN` remains unknown.
3. **PROPOSE** emits a versioned, fenced policy candidate containing only closed
   procedural rule changes, exact incident/evidence commitments, bounded scope
   and cost. A single incident or one-sided replay can never support promotion.
4. **REPLAY/TEST** runs historical snapshots and fault injection. Required
   fixtures include the large-project zero-work misjudgment and TASK-080 audit.
5. **PROMOTE** activates a candidate only when every required replay, safety,
   dependency, cost, privacy, and protected-authority machine gate passes
   against the current policy version and fence. The previous version remains.
6. **MONITOR/ROLLBACK** measures misdispatch, missed dispatch, duplicate work,
   cost, and false positives. A hard-limit regression automatically restores
   the prior policy and advances the fence.

## Epistemic Record And Probe Contract

`lattice.workflow-epistemic-record/1.0` is a fallible, expiring record rather
than a timeless fact. It carries the observation; at least two reasonable
hypotheses when ambiguity exists; confidence and unknowns; supporting evidence
and counterevidence; assumptions and scope; `checkedAt`, `expiresAt`, refresh
triggers, decision, expected/actual signal, falsifier, and next probe.

The hard rule is to seek discriminating evidence safely, not to prefer one
answer. New conflicting evidence lowers confidence; expiry, low confidence,
tool failure, or changed dependencies requests refresh. A reversible probe
declares the hypotheses it distinguishes, cost/risk ceiling, expected and
falsifying signals, and stop condition before it runs. Its result may overturn
the prior decision. No probe may cross protected authority.

## Continuous Suspicion Queue

The foreman maintains a bounded, expiring suspicion queue during normal work,
not only before stopping. Entries are refutable hypotheses across product
behavior, integration/test blind spots, dependencies, Git/worktree topology,
truth-source contradictions, resources/cost, security/authority, and user
value. Ranking uses user value, consequence/risk, freshness, probe cost,
reversibility, and current resource conflict. The next action is the smallest
read-only or reversible probe that can distinguish competing hypotheses.

The queue is not a defect list. Fresh duplicate validation is rejected;
refuted entries retire; expired entries refresh rather than persist as facts;
governance-only doubts are penalized when they would crowd out higher-value
product work. The loop may not manufacture failures, repeat fresh proof, cross
the probe's risk/cost ceiling, or create activity merely to look busy.

## TASK-083 Worktree Topology Case

TASK-083 was created as a nested worktree under TASK-078. The resulting parent
dirty state, unreadable dashboard ticket, and finisher timeout were discovered
by continuous counterexample scanning. The epistemic record preserves competing
hypotheses rather than declaring every timeout a path bug.

The bounded topology probe compares resolved-path identity, approved-root
identity/direct ancestry, nested-worktree ancestry, reparse escape, exact
branch/HEAD, child cleanliness, and parent cleanliness before/after. A proven
nested topology blocks zero-work, healthy, and completion certification and may
recommend only Git-native reversible `git worktree move` after target-absence
and identity checks. Manual deletion/move, reset, clean, and blind recreation
are forbidden. The recorded actual outcome is successful Git-native move,
source absent, target registered/exact/clean, and TASK-078 clean afterward.
A general path invariant still requires another counterexample case before
policy promotion.

## Pre-stop Counterexample Challenge

Before declaring zero work, all waiting, healthy/normal, or ending a foreman
round, the active policy must challenge:

- `UNKNOWN`, idle, quiet, and sidebar/window counts;
- ticket, PLANS, dashboard, Git/worktree, remote, test, and dependency conflicts;
- released dependencies, unopened backlog, and decomposable blockers;
- stale evidence, incorrect paths, failed or omitted tools;
- ready bounded evidence/test/security work and low-cost `gpt-5.6-luna` or
  `gpt-5.6-terra` work that does not cross a resource or authority boundary.

Any counterexample prohibits stopping and produces a typed reconcile action;
an independently safe ready candidate also produces a typed dispatch action.
Quiet is never completion evidence.

Stopping requires a valid `lattice.zero-work-certificate/1.0` bound to the
active policy version/fence and a fresh complete inventory. Every candidate
must have an exact typed dependency, conflict, resource, duplicate, protected,
or cost reason. Missing sources, stale evidence, a ready candidate, unresolved
contradiction, unknown/idle inference, or tool-coverage gap rejects the
certificate.

## User Stories Or System Scenarios

- Replaying the 23 `UNKNOWN`/3 `IN_PROGRESS`/partial dashboard plus TASK-080
  counterevidence rejects zero-work and requests reconciliation/dispatch.
- All windows missing heartbeats triggers an audit; it does not prove there is
  no work.
- A real dependency blocker, worktree/file conflict, duplicate task, or hard
  cost cap remains a dispatch denial.
- A fresh process replays the same observed incidents, active/previous policy,
  fence, and rollback result without model calls or external effects.

## Goals

- Convert workflow mistakes into bounded versioned learning candidates.
- Improve missed-work detection without weakening safety, cost, dependency, or
  writer gates.
- Make stop decisions machine-checkable and reversible.

## Non-Goals

- Autonomous free-form policy editing, model-generated executable rules, or
  self-approval.
- Any change to payment, credential, identity, public-network, force-push,
  default-branch merge, deployment, release, archive, or security authority.
- Worker/process control, scheduler composition, dashboard writes, live
  PostgreSQL, TASK-051, or changes to TASK-079/TASK-078.

## Constraints

- One Gateway, One Truth, One Writer remains unchanged.
- Candidate output is untrusted until machine promotion; reports never grant
  authority.
- Active and previous policy versions plus the current fence are monotonic and
  replayable. Unknown schema/version/fence fails closed.
- Hard cost and protected-authority ceilings are immutable inputs, not learned
  fields.
- Promotion requires multiple distinct incident cases, explicit counterexample
  replay, cost and safety regressions, and rollback proof.

## Module Impact

`workflow-learning` is a new pure module owning closed learning semantics. The
current slice adds no dependencies and no durable owner. Future Task Ledger,
Ports, Postgres Store, and foreman-state integration changes their public
contracts and is blocked on a clean TASK-079 handoff plus versioned amendments.

## Data, Privacy, And Security

- Store only typed codes, bounded identifiers, counters, policy versions,
  fences, and SHA-256/evidence references.
- Untrusted or single-source assertions may be observed but cannot support a
  diagnosis, candidate promotion, certificate, or authority decision.
- Duplicate incident/event identity is idempotent and cannot amplify votes.
- Conflicting, expired, or low-confidence records automatically degrade to
  refresh/unknown; they cannot retain a conclusion by inertia.
- Complete chat, secret-like values, raw commands/output, and arbitrary paths
  reject before observation.

## Compatibility And Migration

Schema 1.0 is additive and standalone. Unknown policy, observation, fixture, or
certificate schema rejects. Future durable events must preserve prior policy
and rollback replay and cannot rewrite historical observations. No database
migration is authorized by this specification.

## Error Cases And Edge Cases

- all windows stale; all windows idle; empty sidebar; stale dashboard;
- ticket says complete while dashboard says unknown/in-progress;
- stale report contains a later-corrected conflict;
- dependency just released; no worker exists for backlog; wrong relative path
  caused a tool failure; a required tool was never run;
- malicious report, duplicated incident, version rollback, stale fence,
  missing gate evidence, cost overflow, and protected-authority mutation;
- monitor regression after promotion and fresh-process replay after rollback.
- two plausible hypotheses, an expired record, counterevidence, and a probe
  result that falsifies the original preferred judgment.

## Acceptance Criteria

- [x] TASK-080 and the large-project zero-work incident replay prohibit stop
  and expose ready safe work.
- [x] `UNKNOWN`, idle, sidebar count, and silence cannot prove no work.
- [x] A zero-work certificate requires fresh exhaustive source coverage and an
  exact reason for every candidate.
- [x] No candidate promotes without all machine receipts; untrusted/malicious
  reports and duplicates cannot contaminate promotion.
- [x] A single case cannot promote; multiple hypotheses coexist; conflicting
  evidence lowers confidence; expiry triggers refresh; a bounded probe can
  overturn the prior decision.
- [x] Policy version/fence, fresh-process replay, prior-version retention, and
  automatic rollback are deterministic.
- [x] Cost ceilings and every protected authority remain immutable.
- [x] Real dependency/resource/duplicate gates remain fail-closed.
- [x] The continuous suspicion queue prioritizes bounded high-value probes,
  expires/refutes hypotheses, rejects fresh duplicates, and prevents governance
  busywork from displacing product work.
- [x] TASK-083 nested topology blocks stop/completion, selects only Git-native
  reversible move, verifies the actual clean outcome, and path policy still
  requires cross-case promotion.

## Verification Plan

| Criterion | Verification method | Expected evidence |
|---|---|---|
| Incident replay | versioned in-repo fixtures plus focused Rust test | stop denied; reconcile/dispatch emitted |
| Certificate | missing/stale/ready/reason matrix | invalid forms reject |
| Learning integrity | trust and duplicate injection | no candidate contamination |
| Epistemic expiry | hypothesis/counterevidence/expiry/probe matrix | unknown preserved; stale conclusions degrade |
| Promotion | multi-case/counterexample/gate/version/fence matrix | only exact complete gate set promotes |
| Recovery/rollback | in-memory checkpoint replay and metric injection | same state; regression restores previous |
| Safety/cost | protected mutation and hard-cap matrix | always rejected |
| Continuous suspicion | ranking, duplicate, expiry, refutation and busywork tests | smallest high-value probe selected |
| Worktree topology | TASK-083 plus nested/reparse fault cases | certification blocked; safe move/outcome verified |

## Human Decisions

The fixed foreman delegation approves this bounded local characterization and a
clean local commit. Push, PR, default merge, deployment, release, archive,
credentials, public exposure, live PostgreSQL, and durable TASK-079 integration
remain unauthorized or blocked.

## Open Questions

The exact Task Ledger event, Writer Lease/fencing tuple, and Postgres schema
binding wait for TASK-079's committed contract. This blocks production
promotion/integration, not the pure characterization checkpoint.
