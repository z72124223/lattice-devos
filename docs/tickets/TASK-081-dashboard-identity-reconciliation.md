---
ticket_id: TASK-081
title: Engineering status identity reconciliation
display_name_zh_tw: 任務 081：工程身分對帳
display_purpose_zh_tw: 讓歷史分支仍能顯示已驗證成果，同時把過期、重複或依賴未解除的交付證據安全地擋在派工與封存之外。
spec_id: SPEC-004
spec_version: 4
module_id: engineering-status-dashboard
constitution_version: 1.3
status: complete
parallel_safe: false
depends_on: [TASK-078]
branch: feature/task-081-dashboard-identity-reconciliation
delivery_remote: origin
delivery_repository: github.com/z72124223/lattice-devos
delivery_push: authorized_non_force_feature_branch
delivery_archive: after_success
allowed_paths:
  - docs/specs/SPEC-004-engineering-status-dashboard.md
  - docs/modules/engineering-status-dashboard/MODULE_CONSTITUTION.md
  - docs/tickets/TASK-081-dashboard-identity-reconciliation.md
  - tools/engineering-status-dashboard/branch-guide.zh-TW.json
  - scripts/export-lattice-engineering-status.mjs
  - test/engineering-status-dashboard.test.js
  - docs/reviews/CODE_REVIEW_TASK_081_2026-08-21.md
  - docs/reviews/ARCHITECTURE_REVIEW_TASK_081_2026-08-21.md
---

# TASK-081 — Engineering status identity reconciliation

## Objective

Repair the read-only engineering-status projection so uniquely anchored,
verified terminal TASK/ISSUE evidence can explain historical branches without
confusing functional outcome with current delivery or archival authority.

## Acceptance conditions

- A unique later committed terminal TASK or ISSUE record with the exact identity
  and branch projects `COMPLETE`/`VERIFIED` for its historical branch.
- Non-current evidence HEAD is `delivery.state: STALE`, cannot dispatch, and
  never implies archive readiness.
- Current malformed, duplicate, removed, uncommitted, or incomparable identity
  evidence stays fail-closed and does not become a historical success.
- Dependency-blocked work remains delivery-blocked even when its outcome is
  complete; cancelled TASK-038 is never recommended.
- Focused dashboard tests, project check, full available verification, review,
  and an authorized non-force feature delivery are complete before archival.

## Non-goals

No change to TASK-050/TASK-075, PLANS/HANDOFF, product runtime, database, MCP,
Hermes, Git mutation in the exporter, deployment, release, or task archival.

## Next action

Deliver the committed branch with the bounded finisher; the separate PLANS
CURRENT marker remains a documented governance blocker because this ticket does
not authorize changing PLANS/HANDOFF.
