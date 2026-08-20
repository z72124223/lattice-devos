---
ticket_id: TASK-095
title: Runtime root-cause diagnostic acceptance
module_id: latticed
constitution_version: 1.1
status: complete
parallel_safe: true
depends_on: []
evidence_subjects: [TASK-033]
branch: feature/task-095-runtime-root-cause-acceptance
delivery_remote: origin
delivery_repository: github.com/z72124223/lattice-devos
delivery_push: authorized_non_force_feature_branch
delivery_archive: keep_open
display_name_zh_tw: 執行期根因診斷驗收
display_purpose_zh_tw: 固化 TASK-095 單次受界限執行期失敗診斷與 TASK-096 觀測修復入口，明確不改寫 TASK-033 終態。
allowed_paths:
  - docs/tickets/TASK-095-runtime-root-cause-acceptance.md
  - docs/reviews/CODE_REVIEW_TASK_095_2026-08-21.md
  - HANDOFF.md
---

# TASK-095 — 執行期根因診斷驗收

## 任務終態與邊界

本票的「complete」只代表一次獨立、受界限的診斷 evidence 已保存；
`TASK-033` 明確仍為 `in_progress`，本票不宣稱交付成功、不改 TASK-033 ticket、
不修產品碼、不再 live、不重建 PostgreSQL，也不封存任一 Codex task。

## 已驗證 evidence

- exact source HEAD：`ef1c3741a862493a7edeea815ef5a7a101aecfcd`。
- 新 fixture：`target/lattice-delivery/9c205fa8acc54aa2881fdba51cb6d68d`；舊
  `dd8f708cf0ac4721b12575ce12f44a1a` 僅唯讀對照，未修改。
- run-owned diagnostic：新 fixture 下
  `evidence/runtime-run-failure-diagnostic.json`，kind
  `LATTICE_DELIVERY_RUNTIME_FAILURE_V1`，child `exit_code=2`，top-level exit 1，
  artifact 256 bytes，stdout 0 bytes、stderr 87 bytes，兩者均未 truncation；未見
  `[REDACTED]` marker，未保存或回顯 raw streams/secrets。安全片段只確認
  `LATTICE_DELIVERY_FAILED`。
- `delivery-run.json`、`delivery-status.json`、`final.json`、runtime-status
  diagnostic 均不存在；nested PostgreSQL root 已 teardown。TASK-095-owned
  process/listener（含 controller/postmaster/child、非 5432）為 0。
- 新 fixture 沒有 `schema/`、`delivery/repo/answer.txt` 或其他未收集的
  run-owned `.log/.out/.err/.tmp`；scripted launcher/server/control metadata 與
  舊 fixture 的允許對照一致，control Git reflog 僅為預期兩筆。

## 診斷結論

confirmed：`apps/lattice-runtime` 在 `composition.rs` 將 terminal Failed receipt
映射為 `LATTICE_DELIVERY_FAILED`，並在 `lib.rs`/`main.rs` 只輸出 enum code；
`cause.stage()` 與 `cause.code()` 在這個邊界遺失。這是已確認的 observability
collapse，並非已確認的下游單一 leaf。

bounded hypotheses：失敗發生於 Codex identity preflight（schema 未建立）或較低
機率的 workspace prepare；identity leaf（schema child、containment、version、
deadline 等）仍未知。由於終態是 Failed 而非 ReconciliationRequired，ambiguous
protocol/EOF/timeout/child-cleanup/job-object 分支不是本次 terminal cause 的證據。

最小離線證偽測試：identity leaf matrix（schema presence + payload-free code）、
terminal receipt mapping 保留 bounded stage/code、以及 scripted schema/server
protocol 對 answer marker 的測試。下一步為 TASK-096 observability repair，owned
paths 建議限定 `apps/lattice-runtime/src/{composition.rs,lib.rs,main.rs}` 及
focused tests；本票不實作。

## wording correction

TASK019 initial/restart live checks 已在 DeliveryRun 前完成；DeliveryRun 失敗前未
發生 delivery restart/status replay。後者不可寫成「restart/replay 通過」。

## 驗證與交付界線

本票以 `npm.cmd run check`、`npm.cmd run verify`、PowerShell AST、`git diff --check`
與 scoped secret scan 驗證文件；合法 finisher/dashboard 若在此 exact source
不可用，必須保持 fail-closed，禁止手動繞過推送。`TASK-033` 終態仍由工頭獨立驗證。
