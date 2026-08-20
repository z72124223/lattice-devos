---
ticket_id: TASK-089
title: Fail-closed evidence-subject governance
spec_id: SPEC-005
spec_version: 7
module_id: engineering-delivery-finisher
constitution_version: 1.6
status: complete
parallel_safe: true
depends_on: [TASK-085]
evidence_subjects: [TASK-050, TASK-075]
branch: feature/task-089-evidence-subject-governance
delivery_remote: origin
delivery_repository: github.com/z72124223/lattice-devos
delivery_push: authorized_non_force_feature_branch
delivery_archive: keep_open
display_name_zh_tw: 追溯對象治理修復
display_purpose_zh_tw: 將追溯與對帳對象獨立於交付相依，避免修復票被未終態對象錯誤阻擋。
allowed_paths:
  - HANDOFF.md
  - docs/contracts/ENGINEERING_PROTOCOL_V1.md
  - docs/specs/SPEC-005-engineering-delivery-finisher.md
  - docs/modules/engineering-delivery-finisher/MODULE_CONSTITUTION.md
  - docs/tickets/TASK-089-evidence-subject-governance.md
  - docs/reviews/CODE_REVIEW_TASK_089_2026-08-21.md
  - docs/reviews/ARCHITECTURE_REVIEW_TASK_089_2026-08-21.md
  - scripts/export-lattice-engineering-status.mjs
  - scripts/finish-lattice-delivery.mjs
  - scripts/check-project.mjs
  - test/engineering-delivery-finisher.test.js
  - test/engineering-status-dashboard.test.js
  - test/project-governance-check.test.js
---

# TASK-089 — 追溯對象治理修復

## 目標

新增 fail-closed 的 `evidence_subjects` 正式欄位，讓 TASK-082／TASK-083
原先追溯的 TASK-050／TASK-075 對象不再被錯誤建模為 `depends_on` 交付前置條件。

## 驗收條件

- `depends_on` 仍只接受交付前必須唯一且成功終態的 TASK；追溯對象不讀取或改變
  被追溯 TASK 的終態。
- `evidence_subjects` 僅接受唯一、合法、已提交的 TASK identity；缺失對象、重複、
  非法格式、與 dependency 重疊、自指或循環一律拒絕。
- finisher 與 read-only dashboard 都從 captured HEAD 驗證並輸出 provenance；
  被追溯 TASK 仍可為非終態，且不會阻止本修復 TASK 交付。
- 不修改 TASK-082／TASK-083 的 worktree、分支或票券。

## 驗證

```powershell
node --test test/engineering-delivery-finisher.test.js
node --test test/engineering-status-dashboard.test.js
npm.cmd run check
npm.cmd run verify
git diff --check
```

## 交付界線

本票僅授權目前 feature branch 的 non-force push、精確遠端 SHA 對帳及工程狀態頁刷新。
不授權 PR、預設分支合併、部署、發布、憑證變更或原生 Codex task 封存；
`delivery_archive: keep_open` 要求交由工頭獨立驗證。
