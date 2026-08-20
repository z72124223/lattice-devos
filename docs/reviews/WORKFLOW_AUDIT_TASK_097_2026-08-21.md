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

## 離線 characterization 結果

在 TASK-097 clean HEAD `f6b75e3183bb69f34e30efd4d0c3920fd0df6019` 的父基底上，
未啟動 Hermes、PostgreSQL、網路 listener 或模型的兩個入口檢查皆 fail-closed：

- `scripts/test-task037-verifier-containment.ps1` 不存在，PowerShell `-File` 回報找不到
  檔案。
- `scripts/run-task037-full-chain-verification.ps1 -HarnessSelfTest` 回報
  `NamedParameterNotFound`，因該 canonical 檔案沒有 `HarnessSelfTest`。

Git 歷史指出 containment script 是本機未推送提交 `c12f6e5` 才加入，完整
admission recovery 到 `9e4b5b4`；後者相對 remote canonical 多五個提交、11 個檔案。
因此目前第一個 confirmed recovery blocker 是「production verifier 不在 canonical
base」，不是 Hermes/Memory/Status 的已確認 leaf root cause，也不授權把該 11 檔案
集合或既有 dirty 修正搬進 TASK-097。

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
