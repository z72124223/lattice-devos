---
ticket_id: TASK-077
title: Local static engineering status dashboard
spec_id: SPEC-004
spec_version: 3
module_id: engineering-status-dashboard
constitution_version: 1.2
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
  - tools/engineering-status-dashboard/branch-guide.zh-TW.json
  - test/engineering-status-dashboard.test.js
  - test/project-governance-check.test.js
branch: feature/task-077-engineering-status-dashboard
---

# TASK-077 — Local static engineering status dashboard

## Objective

Rework the approved SPEC-004 vertical slice from cards into an expandable,
Traditional-Chinese branch tree. The tree must explain branch purpose, derive
parentage from Git ancestry, fail closed when deciding whether a branch may
start new work, and generate a copyable Codex request without mutating Git.

V3 adds plain-language, cost-aware Codex model and reasoning-effort advice to
the selected-branch dispatch panel and its copyable request. The advice must be
deterministic, testable, dated, and explicitly non-authoritative.

The repository validator's exact required engineering-protocol version is also
advanced to 1.0.3 so the mandatory refresh rule and the Chinese purpose-guide
update for every new branch are enforced by the existing `npm.cmd run check`
gate.

## Acceptance conditions

- The SPEC-004 acceptance criteria pass from this ticket's identified branch.
- TASK-051 remains paused and visibly `FAIL`; no live runtime acceptance rerun is
  performed by this ticket.
- Generated artifacts remain outside the repository by default and the source
  worktree is unchanged by refresh.
- No unresolved P0/P1 code, security, architecture, or integration finding.
- Every selectable non-default node has a mapped Chinese purpose, complete
  terminal evidence, a clean worktree, and a verified synchronized remote.
- The user can expand the top-down tree, select an eligible node, enter a work
  description, and copy a Chinese request for Codex to create a separate task
  and branch.
- The completed feature checkpoint is committed and non-force pushed to this
  ticket's feature branch. PR, merge, deployment, release, and public hosting
  remain unauthorized.
- The selected-work panel shows `gpt-5.6-luna` with low reasoning for clearly
  mechanical work, `gpt-5.6-terra` with medium reasoning for everyday work,
  and `gpt-5.6-sol` with high reasoning for high-consequence work.
- Empty or ambiguous descriptions default to the balanced Terra tier; the
  copied request includes the current recommendation, plain reason, escalation
  rule, and a reminder that the model is selected when the new Codex task is
  created.

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

Create the V2 implementation checkpoint, verify it against the actual GitHub
default target, update the durable handoff, non-force push the completed feature
branch, prove remote equality, and refresh the local page after the push.

For V3: implement the approved model guidance test-first, run focused/full
verification and review, update this handoff, then perform the established
non-force feature-branch push and post-push dashboard refresh.

## Completion evidence

The evidence below is retained from V1 and is stale for the reopened V2 work.
New V2 evidence replaces it only after the revised acceptance conditions pass.

- Implementation checkpoint: `89de978404acfefcdb0eec23742657636d4cf16d`.
- Focused dashboard and governance checks: 22/22 passed.
- Independent code/security review: P0=0, P1=0, P2=0, P3=0.
- Architecture review: blocker-free, no ADR or migration required.
- Exact default-branch merge simulation against
  `feature/task-037-full-chain-integration@8828d2b88faece6b399258744eea4ff8d46f0bea`:
  no conflict; combined `npm.cmd run verify` passed 60/60.
- TASK-051 was not rerun and remains explicitly `FAIL` in the live local
  dashboard despite clean Git and passing PR CI evidence.

## V2 completion evidence

- Focused dashboard tests: 19/19 passed, including ancestry failure, snapshot
  freshness, same-commit detached priority, tree reachability, safe output
  replacement, unmapped/dirty/remote-changed denial, and the Windows launcher.
- Focused governance tests: 18/18 passed, including live current-branch,
  `allowed_paths`, Chinese guide entry, decoy-list, mismatch, and English-only
  rejection.
- Final committed feature-source repository verification: 75/75 passed after
  the final narrow message/cleanup repair. The exact combined candidate against the
  actual GitHub default target passed 75/75 with complete temporary-worktree
  cleanup.
- Independent final code/security review: `No findings`, P0=P1=P2=P3=0.
- Architecture review: blocker-free, schema 2.0 is disposable, and no ADR,
  migration, dependency, truth, writer, or authority expansion is required.
- Live V2 refresh: 40 nodes, one root, Git ancestry available, zero unmapped
  nodes, three currently eligible starting points; TASK-051 remains `FAIL` and
  TASK-077 remained non-eligible until this clean completion is pushed.
- Verified implementation checkpoint:
  `c88cc9293f3c521974afe7abe1f74f9e449cfaa4`; integration target:
  `8828d2b88faece6b399258744eea4ff8d46f0bea`.

## V3 implementation evidence

- Live model inventory checked 2026-08-20: current Codex surface exposes the
  Sol, Terra, and Luna tiers; the bundled advisor selected Terra for balanced
  everyday coding.
- Local Qwen3 1.7B is installed and has a basic Traditional-Chinese chat test,
  but it has no LATTICE coding acceptance evidence and is not recommended for
  authoritative implementation.
- RED 1: focused test exit 1 because `recommendCodexSetup` did not exist.
- RED 2: focused test exit 1 because the generated page omitted visible model
  guidance. Review-remediation RED then proved stale advice and the empty-state
  explanation were missing.
- GREEN: dashboard 20/20, governance 18/18, generated-page JavaScript syntax,
  placeholder replacement, `git diff --check`, and full repository 76/76 pass.
- Read-only self-review repaired one P2 stale-catalog risk and one P3
  discoverability gap. Final P0=P1=P2=P3=0; reviewer independence is not proven.
- Final documentation commit, feature push, remote equality, and post-push
  refresh evidence remain terminal delivery steps.
- Exact implementation checkpoint:
  `6ca83afc1eae10cba58e5cb49541d0cdd106c584`.
- Exact integration against actual default
  `8828d2b88faece6b399258744eea4ff8d46f0bea`: no conflict, combined 76/76,
  target-only 0, feature-only 505, temporary-worktree cleanup passed.
- GitHub currently has no TASK-077 PR, CI run, repository ruleset, or protected
  default-target gate. Default merge remains unauthorized and unperformed.
