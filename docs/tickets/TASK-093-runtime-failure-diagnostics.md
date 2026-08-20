---
ticket_id: TASK-093
title: Runtime failure diagnostics
module_id: latticed
constitution_version: 1.1
status: complete
parallel_safe: true
depends_on: []
evidence_subjects: [TASK-033]
branch: feature/task-093-runtime-failure-diagnostics
delivery_remote: origin
delivery_repository: github.com/z72124223/lattice-devos
delivery_push: authorized_non_force_feature_branch
delivery_archive: keep_open
display_name_zh_tw: 執行期失敗診斷
display_purpose_zh_tw: 在 TASK-033 相容的交付包裝器中保留受界限且去秘密化的 child 失敗證據，不改變成功 receipt 或重跑既有 live。
allowed_paths:
  - scripts/run-lattice-delivery.ps1
  - test/run-lattice-delivery-runtime-diagnostics.test.js
  - docs/tickets/TASK-093-runtime-failure-diagnostics.md
  - docs/reviews/CODE_REVIEW_TASK_093_2026-08-21.md
---

# TASK-093 — 執行期失敗診斷

## 目標

修補 TASK-033 top-level delivery wrapper 在 runtime child 非零結束時只回傳
`LATTICE_DELIVERY_RUNTIME_FAILED`、遺失 exit/stdout/stderr 診斷脈絡的缺口。

## 驗收條件

- 非零 child 仍 fail-closed，且不寫入 run success receipt。
- run/status 各自只可在既有 evidence 同層的固定檔名建立一次失敗診斷 JSON；路徑
  受 repository ownership、祖先 reparse 與既存目標檢查保護。
- 診斷含 exit code、分開的 stdout/stderr、截斷旗標；每串流最多 4 KiB，整份
  UTF-8 artifact 最多 32 KiB。無效 UTF-8 replacement 與 NUL encoding 訊號會正規化。
- password、token、credential、Bearer、API key 與 PostgreSQL DSN 不會落入 artifact。
- artifact 以同目錄的 exclusive temporary file 寫入、flush 後不覆寫 move；寫入失敗
  仍只會 fail-closed，絕不偽造 receipt。
- 成功 JSON parse、receipt bytes、timeout、malformed JSON 與不明 child outcome 的既有
  拒絕行為不變。

## 非目標

- 不重跑 TASK-033、PostgreSQL、Graphify 或任何 official Codex live。
- 不更動 Rust、資料庫 schema、Graphify、TASK-033 ticket、PLANS、HANDOFF、保留 fixture
  或 MCP 公開介面。

## 驗證

- `node --test test/run-lattice-delivery-runtime-diagnostics.test.js`
- `npm.cmd run check`
- `npm.cmd run verify`
- PowerShell AST、`git diff --check`、allowlist 與 scoped secret scan。

## 交付界線

本票僅授權目前 feature branch 的 non-force push、精確 remote SHA 對帳與 dashboard
refresh；不授權 PR、合併、預設分支操作、部署、發布、TASK-033 live 重跑或 Codex task
封存。
