# LATTICE

LATTICE 是建立在官方 Codex Harness 上的本機工作控制台。

Codex 負責 agent loop、thread、context、sandbox、工具、MCP、核准事件、
進度與封存；LATTICE 只保存專案、工作、優先度、Codex thread 對應、
使用者驗證與簡短失敗資訊。

## 目前可用的 MVP

```powershell
npm.cmd run control:start
```

接著開啟 [http://127.0.0.1:4317/](http://127.0.0.1:4317/)。

資料預設保存在：

```text
%LOCALAPPDATA%\LATTICE\control\lattice-control.db
```

頁面可：

- 建立專案與工作項目；
- 設定工作優先度；
- 建立或續接同一個 Codex thread；
- 顯示進度與命令／檔案核准；
- 分開記錄 Codex 已結束與使用者已驗證；
- 驗證後封存 Codex thread；
- 關閉並重開 LATTICE 後，從 SQLite 恢復工作與 thread 對應。

Codex 只在按下「開始」或「續接」時連線，不會因開著頁面而呼叫模型。
目前固定使用 `gpt-5.6-terra`。

## 驗證

```powershell
npm.cmd run control:test
npm.cmd run check
npm.cmd test
```

`control:test` 不呼叫模型；它以假的 App Server 驗證保存、重開、續接、
進度、核准、完成、驗證與封存流程。

## 模組策略

下列既有模組保留，但不是啟動 LATTICE 的必要條件：

- Graphify；
- Codebase Memory；
- Project Registry 與 Task Domain；
- 工程狀態頁；
- Hermes、PostgreSQL、Writer Lease、Artifact Store 與舊 Task Ledger。

模組應各自維護、各自測試、各自失敗。新 LATTICE 不再要求 Codex、
PostgreSQL、Graphify、Hermes 與其他模組必須在同一次全鏈路驗收中全部
通過。

## 歷史程式

舊 Rust／PostgreSQL 全鏈路、原有 tickets、plans、handoffs、reviews 與
feature branches 保留作為程式和證據來源。它們不是目前 MVP 的啟動前置
條件，也不應被逐一建立新 TASK 來補齊最新治理格式。

推送、預設分支合併、部署、發布、公開網路與不可逆操作仍需明確授權。
