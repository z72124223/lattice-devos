---
ticket_id: TASK-097
title: TASK-037 production-chain recovery
module_id: hermes-adapter
constitution_version: 1.0
status: in_progress
parallel_safe: false
depends_on: []
evidence_subjects: [TASK-037]
branch: feature/task-097-task-037-production-recovery
delivery_remote: origin
delivery_repository: github.com/z72124223/lattice-devos
delivery_push: authorized_non_force_feature_branch
delivery_archive: keep_open
display_name_zh_tw: TASK-037 正式鏈路復原
display_purpose_zh_tw: 以隔離且可追溯的方式找出 Hermes 到記憶體再到狀態查詢正式鏈路的第一個真實失敗，只修復已證實的失敗，絕不把 canonical-local 驗收冒充 production 通過。
allowed_paths:
  - docs/tickets/TASK-097-task-037-production-recovery.md
  - docs/reviews/WORKFLOW_AUDIT_TASK_097_2026-08-21.md
  - docs/reviews/CODE_REVIEW_TASK_097_2026-08-21.md
  - scripts/run-task037-full-chain-verification.ps1
  - scripts/test-task037-verifier-containment.ps1
---

# TASK-097 — TASK-037 正式鏈路復原

## 目標

從乾淨的 canonical 基底 `8828d2b88faece6b399258744eea4ff8d46f0bea`
重現並界定 `Hermes -> Memory -> Status` 正式鏈路的第一個失敗。TASK-037 僅作
`evidence_subjects` 的追溯對象；本票不改寫其終態，也不將其當成交付前置。

## 邊界

- 不使用或重啟已取消的 TASK-038 ChatGPT tunnel。
- canonical-local 驗收、離線 fixture 與 production E2E 分別記錄，任何一者都不得
  取代另一者。
- 在取得工頭對 run root、動態連接埠、socket、PID 與 marker preflight 的資源授權前，
  不啟動 Hermes、PostgreSQL 或任何 live chain；不得使用 5432，也不碰 TASK-094、
  TASK-033/095 fixtures 或 TASK-051。
- 禁止外部付費模型呼叫；本票不授權 PR、預設分支合併、部署、發布或封存。

## 驗收條件

- 工作樹與基底可精確追溯，且不複製既有 dirty TASK-037 內容。
- 新增的 `HarnessSelfTest` 只可做離線、受界限的本機 child/process、timeout、
  owned-temp cleanup 與 secret/env 非洩漏驗證；不得進入既有 full-chain、Hermes、
  PostgreSQL、OpenClaw 或模型路徑。
- 修復僅涵蓋兩個 verifier script，具 RED/GREEN、PowerShell AST、focused containment、
  diff／secret scan、可用 npm check/verify 與程式碼／安全審查。
- production PASS 只能由新的完整 E2E 證據宣稱；無該證據時狀態維持 `in_progress`。

## 下一步

離線 verifier safety proof 已完成；等待工頭另行授權獨立的 live slice 前，維持
`in_progress`，不啟動 Hermes、PostgreSQL 或 runtime。若取得授權，先提出固定 run
root、動態 port/socket/PID/marker 的 preflight，再保存第一個真實失敗的去秘密化證據。

## 交付界線

本票只授權目前 feature branch 的 non-force push、精確遠端 SHA 對帳與合法
engineering-status refresh。`delivery_archive: keep_open`；即使 checkpoint 成功，
也必須由工頭獨立驗證且本 worker 不封存。
