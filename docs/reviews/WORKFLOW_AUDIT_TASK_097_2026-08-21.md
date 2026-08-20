# TASK-097 工作流程稽核

## 已確認範圍

- 隔離工作樹：`feature/task-097-task-037-production-recovery`。
- 精確基底：`origin/feature/task-037-full-chain-integration` at
  `8828d2b88faece6b399258744eea4ff8d46f0bea`，新工作樹初始為 clean。
- `feature/task-037-operator-entrypoint` 是 clean 但不在該 canonical lineage；
  `feature/task-037-full-chain-publication` 有六個既有未提交檔案，沒有被複製、暫存
  或修改。

## 歷史 production 證據

`PLANS.md` 記錄的 formal acceptance 首個停止點是 Codex broker child 已被啟動、
綁定與身分檢查後，以 `HERMES_PRODUCTION_CHILD_EXITED` 結束；Run 因此 fail-closed，
後續 Memory persistence/readback 與 Status success 均未發生。這是待重新
characterize 的 production-chain failure，不是 production PASS，也不是已證實的
leaf root cause。

## 治理結論

TASK-097 以 `evidence_subjects: [TASK-037]` 追溯舊鏈路，`depends_on: []`，不改動
TASK-037 終態。TASK-038 已取消，並非必要前置且不會重啟。此基底的內建
`npm.cmd run check` 可通過；較新的 TASK-089 外部治理檢查則因歷史基底缺少
Engineering Protocol 1.2 與既有 TASK-033 delivery metadata 而 fail-closed，故在
未整合該治理演進前不可把它當作合法 finisher/exporter 來源。

## 下一步

只執行不啟動 Hermes、PostgreSQL 或 live chain 的 characterization；若第一個可重現
失敗需要 live 依賴，先回報 run root、動態 port/socket/PID/marker preflight 並等待
工頭授權。
