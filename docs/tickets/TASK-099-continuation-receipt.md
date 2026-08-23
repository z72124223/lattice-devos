---
ticket_id: TASK-099
title: Bounded continuation decision receipt
spec_id: SPEC-006
spec_version: 1
module_id: engineering-status-dashboard
constitution_version: 1.3
status: blocked
parallel_safe: false
depends_on: []
evidence_subjects: [TASK-081]
branch: feature/task-099-continuation-receipt
delivery_remote: origin
delivery_repository: github.com/z72124223/lattice-devos
delivery_push: authorized_non_force_feature_branch
delivery_archive: keep_open
display_name_zh_tw: 續作判斷收據
display_purpose_zh_tw: 根據即時證據決定可續作、等待依賴、需要對帳或停止重工。
allowed_paths:
  - scripts/export-lattice-engineering-status.mjs
  - test/engineering-status-dashboard.test.js
  - docs/specs/SPEC-006-continuation-decision-receipt.md
  - docs/tickets/TASK-099-continuation-receipt.md
  - PLANS.md
  - HANDOFF.md
  - tools/engineering-status-dashboard/branch-guide.zh-TW.json
---

# TASK-099 — Bounded continuation decision receipt

## Objective

Add a closed, read-only continuation projection to each engineering-status
snapshot so a new AI window can choose one safe next state without treating
free-text ticket instructions as authority.

## Current blocker

Implementation and local verification are complete on the ticket branch, but
the LATTICE delivery profile reports `LATTICE_DELIVERY_RECONCILIATION_REQUIRED`.
No push, archive, or completion claim may proceed until the server-owned
delivery reconciliation is independently cleared.

## Verification

- `node --test test/engineering-status-dashboard.test.js`
- `npm.cmd run check`
- `npm.cmd run verify`
- `git diff --check`
