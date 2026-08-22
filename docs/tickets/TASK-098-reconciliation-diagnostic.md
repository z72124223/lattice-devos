---
ticket_id: TASK-098
title: Reconciliation diagnostic envelope
module_id: latticed
constitution_version: 1.2
status: in_progress
parallel_safe: false
depends_on: []
evidence_subjects: [TASK-033]
branch: feature/task-098-reconciliation-diagnostic
delivery_remote: origin
delivery_repository: github.com/z72124223/lattice-devos
delivery_push: authorized_non_force_feature_branch
delivery_archive: keep_open
display_name_zh_tw: 對帳安全診斷
display_purpose_zh_tw: 不確定狀態時提供只讀檢查提示，禁止自動重試或變更。
allowed_paths:
  - apps/lattice-runtime/src/lib.rs
  - apps/lattice-runtime/src/mcp.rs
  - apps/lattice-runtime/tests/mcp.rs
  - docs/contracts/RUNTIME_RECONCILIATION_DIAGNOSTIC_ENVELOPE_V1.md
  - docs/modules/latticed/MODULE_CONSTITUTION.md
  - docs/tickets/TASK-098-reconciliation-diagnostic.md
  - PLANS.md
  - HANDOFF.md
---

# TASK-098 — Reconciliation diagnostic envelope

## Objective

Expose a closed, secret-free inspection-only instruction on an existing
reconciliation-required error. The instruction makes no recovery claim and
does not change the two-tool MCP schema.

## Acceptance criteria

- A reconciliation-required MCP error contains exactly
  `reconciliation_action: INSPECT_ONLY` and `retry: FORBIDDEN` in addition to
  its existing safe fields.
- All other errors and all successful results retain their existing fields.
- No returned field contains a path, digest, receipt, workspace, process,
  stdout, stderr, SQL, credential, or arbitrary error text.
- MCP tool enumeration and zero-argument schemas remain unchanged.

## Verification

- `cargo test -p lattice-runtime --locked`
- `cargo fmt --check`
- `npm.cmd run check`
- `npm.cmd run verify`
