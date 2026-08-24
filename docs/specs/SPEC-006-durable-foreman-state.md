---
spec_id: SPEC-006
title: Durable foreman state and takeover acceptance
version: 3
status: approved
approved_by: fixed_foreman_delegation
approved_at_local: 2026-08-21
modules:
  - module_id: foreman-state
    constitution_version: 1.2
  - module_id: task-ledger
    constitution_version: 2.4
  - module_id: postgres-store
    constitution_version: 1.12
  - module_id: lattice-ports
    constitution_version: 2.0
---

# SPEC-006 — Durable foreman state and takeover acceptance

## Problem

The fixed AI foreman currently retains necessary coordination facts only in
chat/automation and a read-only engineering dashboard. A fresh process cannot
verify or reconstruct active, blocked, and next-action state from one durable
LATTICE authority.

## Intended behavior

1. A versioned `lattice.foreman-snapshot/1.0` payload records bounded
   worker/thread identity, task, branch/worktree reference, HEAD, state,
   dependency/blocker reference, latest heartbeat/report digest,
   authority/evidence pointers, and strictly monotonic generation.
   It may separately reference versioned `lattice.foreman-epistemic/1.0`
   expiring epistemic records: observed facts,
   hypotheses, confidence/unknowns, evidence/counterevidence, checked/expiry
   time, refresh trigger, and decision/probe/falsifier pointers. These records
   cannot represent or mutate authoritative snapshot state.
2. Task Ledger owns the fixed `FOREMAN_COORDINATION` stream, the versioned
   `FOREMAN_SNAPSHOT_RECORDED` event, append order, generation/idempotency and
   verified replay. Postgres Store persists only fixed typed scalars bound to
   the matching Ledger event and command in the same serializable transaction,
   after `writer_lease_assert_current_v1`; no child row is independently current.
3. A fresh reader reconstructs active, blocked, archive-ineligible, and
   next-action projections from verified replay without launching a worker,
   running Git, or rereading a chat.
4. A pure watchdog compares untrusted dashboard metadata (`generatedAt`,
   branch, HEAD, outcome) with independently supplied live Git/worktree state.
   It detects all-missed-heartbeat, stale snapshot, old HEAD, dashboard drift,
   and duplicate worker/thread identity.
5. Dashboard output remains a read-only index. TASK-078 exporter changes are
   out of scope; a later adapter may supply its projection through a closed port.

## Non-goals

- Worker/process control, scheduler, Codex archive action, MCP expansion,
  secret migration, TASK-051 acceptance rerun, or heavy/live PostgreSQL tests.
- Full chat, prompt, command, environment, credentials, tokens, raw stderr,
  provider output, or arbitrary path persistence.
- Epistemic learning, promotion, or automatic authority changes; that capability
  is explicitly deferred to TASK-084.

## Module impact

`foreman-state` is a new pure schema/reconstruction/watchdog module.
`task-ledger`, `postgres-store`, and `lattice-ports` require versioned
amendments before their public event/port/schema changes. `lattice-contracts`
uses the already integrated TASK-048 observation types as read-only identifiers.
TASK-049 remains the pure archive-eligibility consumer; it receives no new
authority. The dashboard and delivery finisher remain read-only/non-owning.

## Data, privacy, and security

- Every string/pointer is bounded and allowlisted; full transcript and
  secret-like values reject before hashing/persistence.
- Snapshot write requires the existing exact Writer Lease current head/fence;
  stale/fake/duplicate writer identity rejects.
- Generation, identity, evidence pointer, task, branch, worktree reference,
  HEAD, state, blocker, and authority substitution change the canonical payload.
- Hypotheses are bounded expiring digest references with confidence and
  falsification metadata, never terminal lifecycle truth or free-form text.
- A dependency-blocked task remains retain/open even with an old terminal
  dashboard outcome.

## Edge cases

- Every worker stops refreshing; stale heartbeat is detected.
- Dashboard has an old HEAD or `generatedAt`; watchdog reports drift.
- Restart/replay returns the same projection without side effects.
- Reused worker/thread identity, generation rollback, unknown schema/state,
  missing evidence, malformed pointers, secret/transcript input reject.

## Acceptance criteria

1. Focused tests prove schema versioning, generation ordering, canonical
   substitution rejection, and secret/transcript rejection.
2. An injectable Ledger/port conformance suite proves restart-equivalent
   replay returns active/blocked/next-action without external effects.
3. Watchdog tests prove detection of all missed heartbeats, old HEAD,
   generated/dashboard mismatch, stale snapshot, and duplicate identity.
4. Dependency-blocked snapshots never become archive-ready solely by dashboard
   outcome; TASK-049 retains the window.
5. Focused characterization proves an expiring hypothesis pointer stays
   separate from lifecycle state and rejects non-pointer/free-form hypothesis
   content.
6. Unknown event/payload schema, changed-command retry, stale writer/fence,
   partial child/event state, and migration rollback fail closed.
7. No change touches `scripts/export-lattice-engineering-status.mjs` or starts
   TASK-051 acceptance; live proof uses only a marker-owned disposable cluster.

## Verification plan

| Criterion | Verification method | Expected evidence |
|---|---|---|
| Schema/privacy | focused unit tests | closed payload/rejection matrix passes |
| Fresh reader | in-memory/injectable repository restart fixture | same projection, zero side effects |
| Watchdog | pure live-Git/dashboard fixture tests | required drift findings |
| Governance | `npm.cmd run check`, `git diff --check` | one current ticket and valid docs |

## Human decisions

The user delegated this bounded local implementation through the fixed foreman.
Feature-to-feature integration of TASK-048/049 is authorized. Default-branch
merge, deployment, release, force push, secret/credential handling, and
TASK-051 live acceptance remain unauthorized.
