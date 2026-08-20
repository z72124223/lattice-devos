---
ticket_id: TASK-077
title: Local static engineering status dashboard
spec_id: SPEC-004
spec_version: 1
module_id: engineering-status-dashboard
constitution_version: 1.0
status: complete
parallel_safe: false
depends_on: []
allowed_paths:
  - PLANS.md
  - HANDOFF.md
  - package.json
  - Open-LATTICE-Engineering-Status.cmd
  - docs/contracts/ENGINEERING_PROTOCOL_V1.md
  - docs/specs/SPEC-004-engineering-status-dashboard.md
  - docs/modules/engineering-status-dashboard/MODULE_CONSTITUTION.md
  - docs/tickets/TASK-051-p0-platform-live-acceptance.md
  - docs/tickets/TASK-077-engineering-status-dashboard.md
  - docs/reviews/WORKFLOW_LEDGER_TASK_077_2026-08-20.md
  - docs/reviews/CODE_REVIEW_TASK_077_2026-08-20.md
  - docs/reviews/ARCHITECTURE_REVIEW_TASK_077_2026-08-20.md
  - docs/reviews/INTEGRATION_TASK_077_2026-08-20.md
  - scripts/check-project.mjs
  - scripts/export-lattice-engineering-status.mjs
  - tools/engineering-status-dashboard/index.template.html
  - test/engineering-status-dashboard.test.js
  - test/project-governance-check.test.js
branch: feature/task-077-engineering-status-dashboard
---

# TASK-077 — Local static engineering status dashboard

## Objective

Deliver the approved SPEC-004 vertical slice: live read-only collection, safe
static rendering, a double-click launcher, plain-language branch cards, and the
delivery-protocol refresh hook.

The repository validator's exact required engineering-protocol version is also
advanced from 1.0.1 to 1.0.2 so the new mandatory refresh rule is enforced by
the existing `npm.cmd run check` gate.

## Acceptance conditions

- The SPEC-004 acceptance criteria pass from this ticket's identified branch.
- TASK-051 remains paused and visibly `FAIL`; no live runtime acceptance rerun is
  performed by this ticket.
- Generated artifacts remain outside the repository by default and the source
  worktree is unchanged by refresh.
- No unresolved P0/P1 code, security, architecture, or integration finding.
- The completed feature checkpoint is committed and non-force pushed to this
  ticket's feature branch. PR, merge, deployment, release, and public hosting
  remain unauthorized.

## TDD evidence plan

1. RED: add focused tests for schema/output, worktree collection, explicit
   terminal-state precedence, safe rendering, incomplete sources, remote
   divergence, and source-tree immutability.
2. GREEN: implement the smallest Node standard-library collector and static
   template that passes those behaviors.
3. REFACTOR: simplify evidence normalization and UI rendering without changing
   the tests or module boundaries.

## Verification

```powershell
npm.cmd run check
node --test test/engineering-status-dashboard.test.js
npm.cmd run status:refresh
npm.cmd run verify
git diff --check
```

## Human gate

The user approved this local-only dashboard and the established Codex
finish/push refresh behavior on 2026-08-20. This ticket consumes no authority
for public hosting, PR creation, default-branch merge, deployment, release,
credential mutation, or destructive cleanup.

## Next action

The completed feature branch may be inspected through the local launcher. After
its non-force push, the user may separately choose whether to authorize
integration into the actual GitHub default branch; no merge is implied.

## Completion evidence

- Implementation checkpoint: `89de978404acfefcdb0eec23742657636d4cf16d`.
- Focused dashboard and governance checks: 22/22 passed.
- Independent code/security review: P0=0, P1=0, P2=0, P3=0.
- Architecture review: blocker-free, no ADR or migration required.
- Exact default-branch merge simulation against
  `feature/task-037-full-chain-integration@8828d2b88faece6b399258744eea4ff8d46f0bea`:
  no conflict; combined `npm.cmd run verify` passed 60/60.
- TASK-051 was not rerun and remains explicitly `FAIL` in the live local
  dashboard despite clean Git and passing PR CI evidence.
