# HANDOFF

## Status

`DONE`

## Objective and scope

- 修復 `apps/lattice-control` 的 Codex App Server 接頭生命週期：single-flight
  connect、精確 turn readiness、bounded wait/cleanup、active interrupt、
  fresh-process resume/reconcile、主動 request settlement 與 work-item 冪等。
- Runtime 工作真相：LATTICE／PostgreSQL；未導入新 orchestrator、agent loop、UI 或
  Temporal／LangGraph／OpenHands／Process Compose。
- 本次沒有可綁定一般修復的 durable task submit；唯一可用 intent 是無關的
  `CONTROLLED_CODEX_CANARY`，已明確保留限制且未重播 canary。

## Completed work

- App Server initialize/initialized 改為 single-flight；所有 RPC、notification
  與 server request waiter 都有 timeout、精確關聯及 teardown。
- `thread/start`／`turn/start` RPC 接受只保存診斷；只有同一 thread/turn 的
  `turn/started` 才能把 LATTICE work item 轉成 `running`。
- interrupt 只接受已確認 active turn，並等待同一 turn 的 interrupted/failed
  終態；timeout 才關閉 LATTICE 擁有的 App Server 程序。
- fresh process 先 `thread/resume` 載入 rollout，再用 `thread/read` 對帳；空白、
  非終態或 ID 不符一律 fail closed。
- SQLite compare-and-set 防止跨程序重複建 thread；completed turn 不 replay，
  interrupted/failed 只可 claim 一次有界 retry。
- 未知主動 request 明確拒絕，支援的 approval 有 bounded settlement；完整保存
  `mcpServer/startupStatus/updated` 原始診斷欄位。

## Files changed

- Git diff 保存 Control adapter/service/store/API、測試、兩支 runner、證據與文件。

## Verification

- 真實 Codex App Server：A-F 全部 PASS；兩個 unique thread/turn 均收到
  `turn/started`，確認並行、正常 completed、active interrupt 的 exact
  interrupted terminal、一次 retry、completed 不重做及 fresh-process 對帳。
- E 關先確認舊原生 App Server PID 退出，再由不同 Node PID 的 Control 啟動
  不同原生 App Server PID；兩個新程序在驗收後也確認退出。
- Runtime：`codex-cli 0.144.6`，證據保存實際 binary SHA-256、生成 Schema
  SHA-256、249 筆 notification、36 筆 MCP 診斷與 2 筆 request settlement。
- `npm.cmd run control:test`：42/42 PASS。
- `npm.cmd run verify`：exit 0；全庫 117 PASS、0 FAIL、1 個既有不適用 skip。
- `git diff --check`：PASS。
- 原先兩份失敗證據保持原檔與原 SHA-256，未 reset、clean 或刪除。

## Review

- 獨立 code review：GO；P0=0、P1=0。
- 獨立 architecture review：GO；修復留在既有 adapter/service/store/API 邊界，
  沒有第二份 durable truth 或新增依賴。

## Artifacts

- Durable evidence：`docs/reviews/CODEX_APP_SERVER_LIFECYCLE_ACCEPTANCE_2026-08-26.json`
- 第一輪同程序重建的不足證據已原樣保存為
  `docs/reviews/CODEX_APP_SERVER_LIFECYCLE_ACCEPTANCE_2026-08-26_IN_PROCESS_ATTEMPT.json`。
- 本機驗收資料庫與生成 Schema：證據 JSON 的 `artifacts` 欄位所列
  `.lattice/acceptance/<runId>/`；該目錄維持 gitignored，但檔案與 hash 已保存。
- 原始失敗證據：
  `docs/reviews/CODEX_APP_SERVER_MULTI_AGENT_FEASIBILITY_2026-08-26.json` 與
  `docs/reviews/CODEX_APP_SERVER_MULTI_AGENT_FEASIBILITY_2026-08-26_ATTEMPT_2.json`。

## Delivery status and next action

- 程式交付真相：GitHub 提交、PR 與 CI；本次只建立本機乾淨提交，未 push、
  merge、deploy 或 release。
- 若日後要交付，需另取得 push／整合／部署授權，並以本文件所在提交重新驗證
  remote 與 live gate；本次沒有待修 P0/P1。

## Restart context

- Branch：`product/lattice-control-mvp`。
- 先呼叫零參數 `lattice_runtime_status`；目前 general repair 沒有可用 durable
  task submit，不得用 `CONTROLLED_CODEX_CANARY` 冒充。
- 先讀本文件與 lifecycle acceptance JSON；目前 LATTICE/runtime 狀態仍以
  live tool response 為準，不以本 handoff 取代。
