# LATTICE

LATTICE 是建立在官方 Codex Harness 上的本機 Runtime 與工作控制台。

Codex 負責 agent loop、thread、context、sandbox、工具、MCP、核准事件、
進度與封存；LATTICE Control 保存專案、工作、優先度、Codex thread 對應、
使用者驗證、簡短失敗資訊，以及本機非權威的安裝後觀察收據。

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
- 由 AI 依專案追加保存元件的來源 commit、安裝位置與產物 SHA-256；
- 關閉並重開 LATTICE 後，從 SQLite 恢復工作、thread 對應與安裝收據。

Codex 只在按下「開始」或「續接」時連線，不會因開著頁面而呼叫模型。
目前固定使用 `gpt-5.6-terra`。

### AI 管理的安裝收據

Control 的安裝收據固定標示為 `OBSERVED_AFTER_INSTALL`（安裝後觀察）與
`NON_AUTHORITATIVE`（非部署權威）。收據只能新增，不能修改或刪除；完全相同
的內容再次送出會回傳原收據，不會製造重複紀錄。

使用者頁面不提供手動表單或技術明細。AI 在完成安裝與檔案驗證後使用
`npm.cmd run control:receipt`；這個本機工具會自行計算產物 SHA-256、呼叫
Control API，並依收據 ID 重新讀取剛寫入的觀察紀錄。這只證明紀錄可重讀，
不會把它稱為部署驗證。範例：

```powershell
npm.cmd run control:receipt -- --project-name "LATTICE DevOS" --component lattice-cli --source-commit <40位commit> --artifact <絕對路徑>
```

AI 可透過有界查詢
`GET /api/installation-receipts?limit=50&offset=0` 讀取技術證據；這個資料不會
載入使用者頁面的定期輪詢。

它代表「當時觀察到這個來源版本、檔案位置與產物指紋」，不代表檔案目前仍
存在、服務健康、正式部署成功、GitHub 已驗證或目前仍是最新版。正式 Runtime
與交付真相仍由 PostgreSQL 收據層負責。

## 驗證

```powershell
npm.cmd run control:test
npm.cmd run check
npm.cmd test
npm.cmd run verify
```

`control:test` 不呼叫模型；它以假的 App Server 驗證保存、重開、續接、
進度、核准、完成、驗證、封存，以及 AI 自動記錄／重讀安裝收據的流程。
`verify` 會依序執行專案檢查、Control 測試及其餘 Node 測試，避免 CI 漏掉
Control。

## Runtime 狀態工具

```powershell
cargo run -p lattice-cli -- status
cargo run -p lattice-cli -- status --json
```

這個唯讀工具固定 LATTICE 的核心方向：

| 功能 | 責任 | 故障時的規則 |
|---|---|---|
| LATTICE | 控制與工作流程 | 控制核心失效，Runtime 不可用。 |
| PostgreSQL | 唯一的持久事實與收據 | 不可用時停止持久工作；不能猜測或偽造狀態。 |
| Graphify | 從事實導出的關係記憶 | 進入降級；保留事實，之後可由 PostgreSQL 重建。 |
| Hermes | 反思、疑點與建議 | 進入降級；不得自行覆寫事實或收據。 |

四者是同一個產品，但日常開發採模組驗收與相鄰整合驗收；只有明確的
release-level 檢查才跑完整四段流程。Codex 仍是外部的推理與執行 Harness，
不被重寫進 LATTICE。

### PostgreSQL 健康與交付收據

`lattice-runtime runtime-health` 是非 MCP 的唯讀 Runtime 健康檢查。它使用既有
process-owned PostgreSQL binding，將控制核心、PostgreSQL、交付收據、Graphify
與 Hermes 分開呈現。成功只代表 PostgreSQL 可連線；交付收據固定標示為
`NOT_INSPECTED`，而 `CORE_ONLY` 下 Graphify/Hermes 為 `DEFERRED`，不會被啟動。
它不會建立、讀取或修改交付收據；`lattice_delivery_status` 仍是嚴格的收據
驗證工具。這能避免「資料庫健康、但尚未有交付收據」被錯誤稱為資料損壞。

`lattice-runtime receipt-state` 則只讀取同一綁定的交付收據投影，會明確
回傳 `NOT_STARTED`、`COMPLETED`、`FAILED` 或 `RECONCILIATION_REQUIRED`。
它不會啟動 Codex、Graphify、Hermes 或任何交付效果。

### 分段整合模式

正常的 MCP Runtime 預設為 `CORE_ONLY`：只驗證並回傳 LATTICE 與
PostgreSQL 的核心收據，不啟動 Graphify 或 Hermes。若要進行明確的整合
驗收，才在啟動程序時設定：

```powershell
$env:LATTICE_RUNTIME_INTEGRATION = 'FULL_CHAIN'
```

`FULL_CHAIN` 保留舊有 Graphify + Hermes 附加分析收據，供專門的整合或
發布檢查使用。它不是日常任務完成的前提；未設定或設定 `CORE_ONLY` 時，
Graphify/Hermes 設定都不會被啟動。

## 歷史程式

舊 Rust／PostgreSQL 全鏈路、原有 tickets、plans、handoffs、reviews 與
feature branches 保留作為程式和證據來源。它們不是目前 MVP 的啟動前置
條件，也不應被逐一建立新 TASK 來補齊最新治理格式。

推送、預設分支合併、部署、發布、公開網路與不可逆操作仍需明確授權。
