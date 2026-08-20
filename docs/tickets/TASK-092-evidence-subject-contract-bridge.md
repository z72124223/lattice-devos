---
ticket_id: TASK-092
title: Evidence-subject contract bridge
module_id: engineering-delivery-finisher
constitution_version: 1.6
status: complete
parallel_safe: true
depends_on: []
evidence_subjects: [TASK-050, TASK-075]
branch: feature/task-092-evidence-subject-contract-bridge
delivery_remote: origin
delivery_repository: github.com/z72124223/lattice-devos
delivery_push: authorized_non_force_feature_branch
delivery_archive: keep_open
display_name_zh_tw: 追溯對象合約橋接
display_purpose_zh_tw: 將 1.2 追溯對象合約最小帶入 TASK-082 與 TASK-083 的共同基線，讓追溯不再被誤作交付前置條件。
allowed_paths:
  - docs/contracts/ENGINEERING_PROTOCOL_V1.md
  - docs/tickets/TASK-092-evidence-subject-contract-bridge.md
---

# TASK-092 — 追溯對象合約橋接

## 目標

從 TASK-082 與 TASK-083 的共同已提交基線，最小移植已提交的 1.2
`evidence_subjects` 合約，讓追溯對象與 `depends_on` 交付前置條件維持分離。

## 驗收條件

- 合約內容與 `c6be912fae8e261527a07324833d68632aba8f3e` 的
  `docs/contracts/ENGINEERING_PROTOCOL_V1.md` 1.2 追溯對象規則一致。
- 本票以 `evidence_subjects` 僅記錄 TASK-050 與 TASK-075 的追溯來源；
  `depends_on: []` 不將其終態作為本橋接交付前置條件。
- 不修改 TASK-082、TASK-083、驗證器、匯出器、交付收尾器、產品碼、Cargo、
  PLANS、HANDOFF、branch-guide、PostgreSQL 或 live runtime。

## 驗證

從 `c6be912fae8e261527a07324833d68632aba8f3e` 的外部
`scripts/check-project.mjs`，以本 worktree 作為目前目錄執行；初始 RED 僅為
缺少 1.2 合約與 TASK-092 identity，移植後必須 GREEN。再比對合約雜湊與
`git diff --check`，並執行範圍與秘密掃描。

## 交付界線

本票僅授權目前 feature branch 的 non-force push 與精確遠端 SHA 對帳；不授權
PR、合併、預設分支操作、部署、發布、封存或對 TASK-082／TASK-083 的任何修改。
