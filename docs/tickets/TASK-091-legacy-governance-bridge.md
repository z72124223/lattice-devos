---
ticket_id: TASK-091
title: Legacy governance bridge
module_id: graphify-adapter
constitution_version: 1.1
status: complete
parallel_safe: true
depends_on: [TASK-033]
branch: feature/task-091-legacy-governance-bridge
delivery_remote: origin
delivery_repository: github.com/z72124223/lattice-devos
delivery_push: authorized_non_force_feature_branch
delivery_archive: keep_open
display_name_zh_tw: 舊版治理橋接
display_purpose_zh_tw: 讓合法的 TASK-033 終端修復分支可採用目前治理驗證，不擴張產品或交付權限。
allowed_paths:
  - AGENTS.md
  - docs/contracts/ENGINEERING_PROTOCOL_V1.md
  - docs/tickets/TASK-091-legacy-governance-bridge.md
---

# TASK-091 — Legacy governance bridge

## Objective

Add only the current validator's engineering-entry, completion, delivery, and
ticket-local branch-identity records to the TASK-033 repair lineage.

## Acceptance conditions

- The bridge contains no product, Cargo, plan, handoff, TASK-033, validator,
  exporter, finisher, PostgreSQL, or live-runtime change.
- The protocol and AGENTS instructions preserve fail-closed completion and
  `delivery:finish` routing without granting merge, deployment, release, or
  archival authority.
- This ticket independently identifies the terminal repair branch with safe
  delivery metadata and Traditional-Chinese display metadata.

## Verification

Run the external validator committed at
`e34bc9bfcf18c71e771f704d50128e1fbeba53ea` with this worktree as its cwd,
then run `git diff --check` and a scoped credential scan. The initial RED recorded
the missing protocol, AGENTS entry/completion/delivery guards, and TASK-091
ticket identity. Any unchanged legacy TASK-033 metadata failure remains
outside this ticket's allowed paths and is reported to the foreman.

## Delivery boundary

This ticket authorizes only a non-force feature-branch push. It does not
authorize a PR, merge, deployment, release, default-branch operation, or
native Codex task archival; `delivery_archive: keep_open` reserves final
verification for the foreman.
