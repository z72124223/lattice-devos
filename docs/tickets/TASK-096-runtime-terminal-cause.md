---
ticket_id: TASK-096
title: Runtime terminal cause preservation
module_id: latticed
constitution_version: 1.1
status: in_progress
parallel_safe: true
depends_on: []
evidence_subjects: [TASK-033]
branch: feature/task-096-runtime-terminal-cause
delivery_remote: origin
delivery_repository: github.com/z72124223/lattice-devos
delivery_push: authorized_non_force_feature_branch
delivery_archive: keep_open
display_name_zh_tw: 執行期終態原因保留
display_purpose_zh_tw: 在不洩漏 payload 或秘密的前提下，將已驗證的 delivery 終態 stage 與 cause code 傳至 CLI 和 MCP 錯誤封包。
allowed_paths:
  - apps/lattice-runtime/src/composition.rs
  - apps/lattice-runtime/src/lib.rs
  - apps/lattice-runtime/src/main.rs
  - apps/lattice-runtime/src/mcp.rs
  - apps/lattice-runtime/tests/composition.rs
  - apps/lattice-runtime/tests/mcp.rs
  - docs/contracts/RUNTIME_TERMINAL_FAILURE_ENVELOPE_V1.md
  - docs/tickets/TASK-096-runtime-terminal-cause.md
  - docs/reviews/CODE_REVIEW_TASK_096_2026-08-21.md
  - docs/reviews/ARCHITECTURE_REVIEW_TASK_096_2026-08-21.md
---

# TASK-096 — Runtime terminal cause preservation

## Objective

Preserve a verified known `DeliveryPortError` stage and stable cause code from
the TASK-032 terminal receipt through the composition, compatibility CLI, and
bounded MCP tool error envelope. The output is closed and payload-free.

## Acceptance criteria

- A known `FAILED` terminal receipt preserves a closed `stage` and
  `cause_code`; source details, paths, stdout, stderr, payloads, SQL, and
  secrets are absent.
- Unknown, invalid, or receipt-mismatched cause codes fail closed without
  echoing their value.
- Reconciliation and ambiguous semantics remain unchanged; no error mapping
  can produce a completed result.
- Success receipt bytes and behavior remain unchanged.
- The matrix covers every existing Codex identity/process leaf plus a
  workspace early failure and all delivery stages.
- Offline scripted fixture characterization proves schema mode creates only
  the schema marker and Server mode creates `answer.txt`; it does not start
  PostgreSQL or a full delivery acceptance.

## Non-goals

- PostgreSQL, Graphify, Codex adapter product behavior, PowerShell wrapper
  behavior, TASK-033/TASK-095 ticket changes, official live delivery, identity
  root-cause claims, deployment, release, merge, or task archival.

## Verification

- `cargo test -p lattice-runtime`
- `cargo fmt --check`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test --workspace --locked`
- `npm.cmd run check`
- `npm.cmd run verify`
- scoped secret scan and `git diff --check`
