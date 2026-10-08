# LATTICE — Codex App 的三核心後台

> **Claude／AI 閱讀入口：[CLAUDE.md](CLAUDE.md)** · [共用工程規則](AGENTS.md)
>
> **一眼看懂目前版本**：LATTICE 控制＋PostgreSQL＋Graphify 三核心，Hermes 已退役；Rust／PostgreSQL 仍在使用。
> Codex App 是操作介面；全產品並非鎖定單一模型。已有 Windows 安裝候選包，尚未選定 LICENSE。
> Claude Code 可透過 `CLAUDE.md` 載入共用規則；只讀 GitHub 網頁的 AI，請直接開啟上方導讀連結。

**讓 Codex 的工作有正式任務、可查證的紀錄，以及程式碼關係圖譜。**

LATTICE 是為 **Codex App** 設計的本機工作後台。你在 Codex 裡提出需求、查看進度與核准操作；
LATTICE 透過 MCP 提供任務與專案查詢，將正式工作、決策和驗證結果保存在 PostgreSQL，
並用 Graphify 分析程式碼關係。重新連線後，可以用正式任務編號讀回既有狀態與證據。

[下載 Windows 安裝包](https://github.com/z72124223/lattice-devos/releases/download/v2.0.1-rc.7/LATTICE-Setup-v2.0.1-rc.7-windows-x64.exe) · [安裝指南](INSTALL_WITH_CODEX.md) · [版本與驗收附件](https://github.com/z72124223/lattice-devos/releases/tag/v2.0.1-rc.7)

> **目前版本（2026-09-22）**：三核心產品為 **2.0**；最新 Windows 完整安裝包為 **v2.0.1-rc.7 候選版**，約 **607 MB**。
> 已通過獨立 Windows Server 2025 安裝驗收；普通 Windows 10／11 首次啟用 WSL、UAC／重開機及 Codex Desktop 登入仍待乾淨真機驗證。

> **閱讀版本**：目前產品文件在預設分支 [`product/lattice-control-mvp`](https://github.com/z72124223/lattice-devos/tree/product/lattice-control-mvp)。
> 分支名稱沿用早期名稱；評估目前功能請核對本文、[產品規則](AGENTS.md)與[安裝指南](INSTALL_WITH_CODEX.md)的同一版本。
> 若網頁摘要與本文不同，可直接讀取[目前 README 原文](https://raw.githubusercontent.com/z72124223/lattice-devos/product/lattice-control-mvp/README.md)，並用[提交紀錄](https://github.com/z72124223/lattice-devos/commits/product/lattice-control-mvp/README.md)核對日期與版本。

## LATTICE 能幫你做什麼？

| 你想知道的事 | LATTICE 提供的能力 |
|---|---|
| 這個工作屬於哪個專案、目前做到哪裡？ | 正式任務身分、狀態，以及父子任務與相依關係查詢 |
| 先前做過什麼決定，依據在哪裡？ | PostgreSQL 保存決策、來源與可讀回的工作紀錄 |
| 修改這段程式可能影響哪些地方？ | Graphify 分析程式碼，再查詢已保存的關係圖譜 |
| 這次任務到底有沒有查圖譜？ | Runtime 自動記錄受觀測的分析、重用與查詢，可按任務讀回次數、結果筆數及大小 |
| 所謂「完成」有沒有實際驗證？ | 支援由 Runtime 執行指定 Node 測試、保存證據並匯入正式任務結果的本機驗證流程 |

任務完成狀態、測試通過、GitHub 合併、部署與真人驗收，各自需要對應證據；
單一 `COMPLETED` 或健康檢查結果不能代替全部交付階段。

## 三核心如何分工？

| 核心 | 責任 |
|---|---|
| **LATTICE 控制** | 正式工作身分、專案與任務關係、授權、決策、驗證結果及可重播狀態 |
| **PostgreSQL** | 唯一權威資料來源，持久保存工作、決策與證據 |
| **Graphify** | 從程式碼產生可重建的關係資料，供查詢呼叫關係與影響範圍 |

**Codex App 是使用者唯一操作介面**，負責模型推理、執行工具、工作視窗、進度、核准與封存。
LATTICE 在背景提供資料與工具，不另建通用 AI 代理迴圈，也沒有獨立網頁或桌面操作平台。
工作樹關係及圖譜資料仍保存在後台。

**Hermes 反思功能已永久退役**，沒有登入、設定或命令可以重新啟用。
主產品使用 `LATTICE_RUNTIME_INTEGRATION="GRAPHIFY"`。
有效值只有 `CORE_ONLY` 和 `GRAPHIFY`；舊名稱 `FULL_CHAIN`、`GRAPHIFY_HERMES` 已拒絕，請勿沿用舊版設定。

## 舊版說明與目前產品的差別

早期的 [2026-08-24 README](https://github.com/z72124223/lattice-devos/blob/86c25bf4f2731c83521cfa32d44437cf06a03bdf/README.md)
同時描述了網頁 MVP、固定 `gpt-5.6-terra`、包含 Hermes 的四元件，以及「舊 Rust／PostgreSQL 全鏈路」的歷史程式。
這些是**當時版本的範圍**；引用舊分支、舊提交或搜尋摘要時，不能直接套用到目前產品。

| 項目 | 目前範圍 |
|---|---|
| 產品用途 | 在 Codex App 背後保存正式任務、決策與可查證結果，並提供程式碼關係查詢；完成證據管理是其中一項能力。 |
| 核心與介面 | LATTICE 控制、PostgreSQL、Graphify 三核心；Hermes 已退役，使用者在 Codex App 操作。 |
| Rust／PostgreSQL | Rust Runtime 與 PostgreSQL 儲存層仍是現行實作；保留的舊全鏈路驗收文件不代表這兩者已停用。 |
| 模型 | 沒有全產品固定使用 `gpt-5.6-terra` 的規則；原生工作與歷史受管流程的模型約束不同，詳見下文。 |
| 安裝 | 已有包含依賴的 Windows x64 EXE 候選包；一般使用者不需先學會 Rust 或 Node.js 才能依指南安裝。 |
| 平台與成熟度 | 目前交付目標為 Windows／WSL；尚無完整 macOS 或 Linux 桌面安裝驗收。2.0 三核心產品與 rc.7 安裝候選包的驗證範圍須分開判讀。 |

可用模型須以實際執行入口與 Codex 帳號能力為準。現行原生工作 claim 路徑續用已保存的模型，
沒有既有模型時預設 `gpt-6-astra`；[專案規則](AGENTS.md)也要求保留既有對話模型。
部分歷史受管執行與語意審查合約仍包含 `gpt-5.6-terra` 或限定模型列舉，
不能把它們說成全產品的固定模型，也不能宣稱所有路徑都支援任意模型。
可查核 [Runtime 模型選擇](apps/lattice-runtime/src/composition.rs)、[歷史受管審查](apps/lattice-runtime/src/managed_semantic_reviewer.rs)，
以及目前使用中的 [Rust Runtime](apps/lattice-runtime/)與 [PostgreSQL 儲存層](crates/lattice-postgres-store/)。

安裝觀察收據的 `NON_AUTHORITATIVE` 邊界仍然有效：它能記錄當時的觀察，不能單獨證明部署成功。
同樣，公開原始碼、安裝測試、正式任務完成與真人使用驗收各有不同範圍；相關限制見下方驗收與授權說明。

## 下載與安裝

**[直接下載 LATTICE rc.7 Windows x64 安裝包（607 MB）](https://github.com/z72124223/lattice-devos/releases/download/v2.0.1-rc.7/LATTICE-Setup-v2.0.1-rc.7-windows-x64.exe)**

1. 在自己的 Windows 電腦安裝 Codex App，並登入自己的帳號。
2. 下載並雙擊 `LATTICE-Setup-v2.0.1-rc.7-windows-x64.exe`。
3. 依畫面處理 Windows 授權、必要的重開機，以及設定衝突選擇；需要重開機時，之後再雙擊同一檔案。
4. 安裝完成後重新開啟 Codex，依[安裝指南](INSTALL_WITH_CODEX.md)實際讀回 Runtime、任務與圖譜結果。

安裝包包含三核心，以及固定版本的 **Python、Git、Node、PostgreSQL、Graphify 依賴、專用 Ubuntu WSL 映像和必要 Microsoft 執行元件**。
一般使用者不需自行編譯或另外尋找這些依賴；Windows 的 WSL／虛擬化條件仍須符合要求。
沒有指定專案時，安裝器會用專用歡迎範例驗收；自己的專案需另行登記及分析。

安裝器也會設定 Codex 的 LATTICE MCP 連線與全域工作指示，並檢查既有技能、工作流程和 MCP／插件可能重複的項目。
可處理的重複項目須使用者同意才備份停用；未知格式或僅名稱相似的項目只提示確認。
可攜偏好與必要環境設定會保留適用項目，**不複製登入身份、token、密碼或帳戶資格**；換電腦仍需登入自己的 Codex。

不熟悉技術設定，可以把這段交給自己的 Codex：

```text
請依照 https://github.com/z72124223/lattice-devos/blob/product/lattice-control-mvp/INSTALL_WITH_CODEX.md
協助安裝目前的 LATTICE 三核心完整候選包，先核對版本、摘要與電腦環境。
保留我的既有設定，讓我處理 Windows 授權、重開機與設定衝突選擇。
完成後實測任務、Graphify 圖譜及重新連線讀回；未通過的項目請如實回報。
```

安裝包的固定來源、SHA-256、進階手動安裝，以及備份／還原方式，見[安裝指南](INSTALL_WITH_CODEX.md)與[Runtime 操作文件](docs/customer-runtime.md)。

## rc.7 新增：Graphify 使用紀錄

現在可以查詢這個任務**已被 Runtime 記錄的圖譜分析與查詢次數**，並讀回結果大小及耗時。

| 操作 | 實際行為 |
|---|---|
| `graphify-refresh` | 產生新分析或重用符合條件的既有分析 |
| `lattice_code_relations` | 查詢 PostgreSQL 已保存的圖譜；不自動建立缺少的分析 |
| `lattice_graph_usage` | 讀回已記錄的分析、重用、查詢、失敗、結果筆數、位元組與耗時 |

Runtime 在受觀測操作開始與結束時寫入 PostgreSQL，並記錄實際分析／查詢函式的進入次數。
將正式 `task_ref` 傳給 refresh 或關係查詢，即可綁定任務；省略時記為未綁定。

- **`UNKNOWN`**：沒有觀測紀錄，不能當成從未使用過 Graphify。
- **`INCOMPLETE`**：有開始但沒有結束紀錄，統計不完整。
- **`OBSERVED_CALLS_ONLY`**：已記錄的呼叫完成；不代表整個 Codex 對話或所有外部程式都被監測。

`lattice_task_submit` 與 `lattice_task_status` 不會因為管理任務就自動查圖譜。
安裝時的「全域掛鉤」是寫入 `AGENTS.md` 的工作指示，**目前沒有「未查 Graphify 就拒收任務」的硬性關卡**。
`CORE_ONLY` 也不是所有圖譜入口的全域禁用開關；尚未有完整任務全程零 Graphify 呼叫的驗收證據。

健康檢查、使用紀錄與工作完成是不同證據。紀錄能證明受觀測的呼叫及結果，不能證明 AI 理解或採用了資料。
原始碼位置、計數方式及測試範圍見 [Graphify 固定版本查核入口](docs/graphify-execution-evidence.md#固定版本查核入口)。

## 已驗證的範圍

| 驗收 | 實際結果與證據 |
|---|---|
| 完整 EXE 獨立安裝 | Windows Server 2025 約 **6 分 15 秒**通過；三核心、6 筆圖譜／MCP 讀回、Runtime／PostgreSQL 重啟、全域規則、偏好與必要元件載入均通過。[安裝驗收](https://github.com/z72124223/lattice-devos/actions/runs/35595328948) |
| 正式 Graphify 新分析 | 正式安裝啟動程式分析新 commit 約 **43.094 秒／9 筆**；原生流程另驗證新分析、重用、任務綁定查詢，以及 PostgreSQL 重啟後紀錄一致。[去敏證據](https://github.com/z72124223/lattice-devos/releases/download/v2.0.1-rc.7/graphify-release-499b30a-acceptance-public-20260921.json) |
| 備份與還原 | 先前版本已驗證同主機、同帳號的單專案還原；不擴稱跨帳號或多專案還原已通過。[歷史驗收附件](https://github.com/z72124223/lattice-devos/releases/tag/v2.0.1-rc.6) |
| 原始碼檢查 | Windows 安裝測試、npm 驗證、Rust 格式、PostgreSQL 測試及指定範圍的嚴格 Clippy 通過。[程式驗證](https://github.com/z72124223/lattice-devos/actions/runs/35624848630) |

普通 Windows 10／11 **首次 WSL 啟用、UAC／重開機與 Codex Desktop 登入**仍未完成乾淨真機驗收，完整安裝包因此保留候選版標示。
非預設專案搬移或還原後，舊圖譜可能需要重新分析。曾中斷的測試紀錄仍保留，不因後續成功而抹除。

PostgreSQL 故障時，正式工作不可用；Graphify 可獨立降級及修復。
`PREPARED` 只代表配置已存在，不能當成分析或交付已成功。

## 文件與維護

- [安裝指南與必要環境](INSTALL_WITH_CODEX.md)
- [Runtime 啟用、更新、備份與還原](docs/customer-runtime.md)
- [Graphify 程式碼、行為及驗收證據](docs/graphify-execution-evidence.md)
- [目前產品方向](AGENTS.md)與[工程協定](docs/contracts/ENGINEERING_PROTOCOL_V1.md)

<details>
<summary>從原始碼操作與既有本機 API</summary>

Node.js 需要 24.15 或更新版本；Runtime 使用儲存庫固定的 Rust 工具鏈和 PostgreSQL。

```powershell
npm.cmd ci --ignore-scripts
npm.cmd run control:test
npm.cmd test
npm.cmd run check
```

既有本機 API 可用 `npm.cmd run backend:start` 啟動，只監聽 `127.0.0.1:4317`，不提供網頁。
本機 locator 與非權威安裝觀察仍保存在 `%LOCALAPPDATA%\LATTICE\control\lattice-control.db`；
正式任務與證據以 PostgreSQL 為準。

```powershell
npm.cmd run control:project -- register --name "My Project" --path "C:\absolute\project"
npm.cmd run control:project -- read --project-name "My Project"
```

目錄登記後，正式專案身分仍須由 Runtime 的 PostgreSQL Project Registry 綁定。
更新既有安裝前先備份設定與資料庫，依 Runtime 文件執行升級；舊 Codex 連線需重新連線才會使用新版本。

### Node 相對模組缺失的唯讀診斷（LH-DELIVERY-01）

明示專案、匯入檔及來源快照後，可使用獨立入口；不連線 Control／DB、不修復或執行專案程式：

```powershell
node --experimental-vm-modules apps/lattice-control/src/relative-module-diagnostic-cli.mjs C:\work\diagnosis-request.json
```

`diagnosis-request.json` 的形狀如下；路徑皆為明示絕對路徑，雜湊是原檔完整 bytes 的 SHA-256：

```js
{ projectRoot, importer, evidenceFile, evidenceSha256,
  context: { projectId, taskId, projectScope: "matched", authority: "authorized",
    lifecycle: "active", freshness: "current", circuitOpen: false, validUntil } }
```

`validUntil` 是呼叫端設定的當下有效 ISO 時間。`evidenceFile` 必須是以下完整快照，
`event` 保留來源事件的 id、type、status、command、aggregatedOutput、exitCode：

```js
{ schema: "lattice.relative-module-evidence.v1",
  subject: { projectId, taskId, projectRoot, importer, importerSha256 }, event }
```

**呼叫端／來源轉接器仍須獨立核對原生事件、正式身分與當下讀取授權。**
此入口只核對資料、檔案和路徑一致性；JSON、雜湊及 `authorized` 宣告不是權限證明。
所有結果皆為 `advisoryOnly=true`、`adopted=false`，兩項 `nativeProvenanceVerified`／
`authorizationVerified` 固定為 false。非適用狀態排除，來源漂移、缺證據或不支援形式會棄答，
回傳具體原因；選中時回傳 importer、specifier、目標與最近既存父層及唯讀提示。

僅以 Node 內建 V8 parser 解析 `.mjs`／`.js` 的靜態 ESM 相依（含 re-export），不 link／evaluate；
註解／字串不算匯入，dynamic import、CJS／TypeScript、import attributes、URL 編碼／query 等保守棄答。
缺少實驗旗標時回傳 `PARSER_UNAVAILABLE`。專案根、匯入檔、目標及缺失目標的既存祖先都拒絕連結跳轉；
檔案重驗是當次觀察，不是敵意並行檔案系統的原子隔離保證。
聚焦檢查：`node --experimental-vm-modules --test apps/lattice-control/test/relative-module-diagnostic.test.mjs`。
本機 Windows／Node 24.16.0 實測 42 項中 41 項通過、1 項因檔案符號連結建立回傳 `EPERM` 略過；
三種 junction（目錄連結）邊界實測通過。Linux／macOS 尚未實測。

一次受控流程（LH-DELIVERY-02）的[可攜結果](experiments/life-harness/delivery-02-result.json)已記錄：
原生缺失模組失敗 → 此唯讀入口定位 → Codex 僅新增允許的檔案 → 相同命令／目錄成功，
並核對預先固定的 `LH-DELIVERY-02 total=42` 輸出及程式內功能斷言。
修復後重查已棄答 `TARGET_EXISTS_NOW`，真實成功事件被排除，獨立控制輸入的來源漂移也被拒絕。
結果包含修復前後內容／雜湊、明示的原生形狀轉接及原始事件雜湊；本機私有來源索引位於
`.lattice/life-harness/delivery-02/run-20260924-01/`，可只讀核對，無須重做修復。
此為一次預先安排的工程可行性流程，不證明泛化、生產收益或診斷的因果改善；未接入正式恢復流程。

Control 結果接入（LH-DELIVERY-03）：已有唯讀入口
`GET /api/formal-work/<taskRef>/diagnostic`，查詢欄位為 `projectId`、`claimId`、`threadId`、
`turnId`、`failureItemId`，以及可選的 `diagnosticItemId`。它只從 PostgreSQL 新快照及已擁有的 Codex
原生回合讀回結果；不開啟專案／證據檔、不執行診斷或修復、不寫正式狀態。

由 Codex 在當次原生工具授權內產生診斷：在前述 request 加入 `receiptBinding`，內容是
`{ projectId, taskRef, claimId, threadId, turnId, inputId, failureItemId }`，`context.taskId`
須等於正式 `taskRef`，`validUntil` 設在五分鐘內。完整 request bytes 計算 SHA-256 後，
使用 [`diagnosticReceiptCommand(requestPath, requestSha256)`](apps/lattice-control/src/relative-module-receipt.mjs)
取得交給原生 shell 的完整內層命令；CLI 的選用參數 `--receipt <request-sha256>` 核對輸入 bytes 並輸出封套。
原失敗 event 保留原生 `id/type/status/command/aggregatedOutput/exitCode`，不能用模型回覆代替。
封套模式的命令會帶所需 VM Modules 旗標；Control 本身不解析專案原始碼。
目前只接受 Windows 磁碟絕對路徑 `pwsh.exe`／`powershell.exe`、單一 `-Command` 引數的
原生 v2 shlex 完整顯示字串，並要求唯一 `unknown` command action 與完整內層命令一致。
額外 shell 引數、前後附加命令、管線、裸命令及其他 shell／引號編碼均拒收；路徑含引號、
智慧引號、`$`、反引號、`!` 或 `^` 也不支援。shell 路徑元件限字母、數字、空白、`._()-`。
這是受限格式核對，不會執行收到的命令；失敗事件摘要仍涵蓋未拆解的原生 shell 包裝。

入口前後核對目前正式 project snapshot／claim／派工 marker、同一連線世代與 session，
以及同一活動回合中先失敗、後診斷的指定 command item。錯身分、撤銷／完成／封存、
待核准、拒絕／熔斷、過期、來源漂移或格式／大小不符均拒收，不會重新派工。
成功只表示 `nativeClaimBindingVerified=true`；生產者、輸入檔 bytes 與診斷內容並未由
Control 獨立驗證，對應旗標保持 false，內層 `advisoryOnly=true`、`adopted=false` 與兩個
未驗證權限／來源旗標也保持原值。檔案狀態仍只是 CLI 當次觀察，不是 API 讀取時的最新檔案狀態。
前後核對是當次觀察，不是 PostgreSQL 與 App Server 之間的原子快照。
本機適配器／HTTP 契約測試使用隔離 doubles；另於 2026-09-27，以固定 `e2bb735`
完成 LH-DELIVERY-03-LIVE 的真實正式 claim／同回合診斷綁定：HTTP 200、4369 ms，
錯誤 selector 回 409。該 fixture 已中斷，保留 DRAFT／result_digest=null，不能視為正式結案。
LH-V1-E2E-01 已於 2026-10-03 完成同一受控任務的真實失敗、收據與 HTTP 綁定、
單檔修復、相同命令成功及獨立驗收；Runtime 固定測試 6/6 通過，正式保存後由新程序
讀回 `COMPLETED` 與非空 result digest。先前中止與失敗證據保留；此受控案例不等於
一般交付已驗證。歷史執行曾釘選另一 helper root，日常交付須使用下述同根入口。

</details>

## 授權

目前尚未選定 `LICENSE`。GitHub 公開可見性與 `git clone` 功能不等於開源授權；第三方依賴保留各自的授權與散布文件。
目前提供本機使用方式，沒有公開雲端服務。

## 聯絡作者

- 作者／維護者：[z72124223](https://github.com/z72124223)
- 聯絡信箱：[z72124223@gmail.com](mailto:z72124223@gmail.com)

歡迎來信交流 LATTICE、詢問安裝與使用問題，或洽談合作及授權。

## 本機診斷輸入準備（LH-LOCAL-PREP-01）

```powershell
node --experimental-vm-modules --disable-warning=ExperimentalWarning apps/lattice-control/src/relative-module-prepare-cli.mjs C:\work\prepare-input.json
```

輸入 JSON 明示 `projectRoot`、`importer`、`failureFile`、`failureSha256`、
`outputDirectory` 與前述診斷使用的完整 `context`。路徑皆須絕對路徑；`failureFile`
是單一原生 `commandExecution` item 的 JSON 匯出（不是通知封套或整段對話），SHA-256
核對匯出檔原始 bytes，保留完整 shell wrapper。context 的 projectId/taskId、authority、
projectScope、lifecycle、freshness、circuitOpen、validUntil 必須由呼叫者明示，不從日誌推定。

預設 `mode:"diagnostic"`；需要收據才明示 `mode:"receipt"` 及完整 `receiptBinding`
七欄（projectId、taskRef、claimId、threadId、turnId、inputId、failureItemId），其中
projectId/taskRef 必須符合 context，failureItemId 必須符合事件。收據期限最多五分鐘，
入口不補造身分或延長期限；其餘 claim/thread/turn/input 關係仍須由正式原生入口核對。

只在 preflight 選中相對缺失提示後，於**明示、尚不存在、父目錄已存在**的 outputDirectory
建立 evidence.json/request.json；可以是專案外的明示封存目錄，但不得穿越 symlink/junction，
不得與缺失目標碰撞。拒絕既存目錄／覆寫，失敗保留部分產物，另選新目錄才能再準備。
stdout 只回摘要、檔案雜湊與精確命令；準備成功 exit 0，其餘 exit 1，不傾印原始日誌。
函式入口為 `prepareRelativeModule(input)`；不啟動子程序、網路、DB、模型、修復或重試，
回傳命令須由 Codex 另行執行。來源仍是 caller-supplied／context caller-asserted，
native provenance／authorization 未驗證、adopted=false；路徑與 bytes 重核不是原子檔案系統沙箱。

目前涵蓋 Node 靜態相對模組缺失，收據入口接受已核對的 Windows 與 Linux 原生命令格式；
Linux 的正式任務與診斷閉環仍須在交付主機完成驗收。手動來源及診斷入口仍可使用；
日常 Control 啟動另為其擁有的 EXECUTION 活動回合啟用失敗候選通知，
經來源守衛核對後透過原生 `turn/steer` 提醒同一回合的 Codex 執行診斷。
Control 不執行診斷命令或修復；修復仍須符合原工作允許的檔案與成功條件。
正式工作仍須走既有 executor → independent verifier → Runtime 固定測試／結果匯入，
重新讀回 COMPLETED、非空 result_digest 與 completion_verified=true 才算結案。
JEV 已排除於正式交付，Control 不再提供 JEV 建議入口；歷史實驗保留於
[experiments/life-harness](experiments/life-harness/README.md)。
Linux 雲端的安裝、啟動、認證與 Graphify 限制見 [雲端設定](docs/cloud-setup.md)。
一般交付仍待當輪驗證，本機驗證與 push／merge／部署／release 分開。

### 從 Codex App 指派受管程式工作

`node apps/lattice-control/src/formal-work-client.mjs projects` 讀取已登記的專案；
`list --project <projectId>` 先找可續接的工作。新工程工作以 `create --input <JSONfile>`
傳入 `projectId`、固定 `clientRequestId`、`objective`，可附 `title` 與 `successCriteria`。
沿用回傳的正式 `taskRef`，再執行 `start --project <projectId> --task <taskRef>`。
`status` 使用相同參數讀回結果；重複 create 必須沿用原請求身份，不暗中重新派送。
這個客戶端只連 loopback 上正在執行的同一 Control，不建立第二個服務或所有權表。

現有 FormalTaskService 會建立其持有的原生 Codex 執行回合及隔離工作目錄；
目前 App 對話負責指派與呈現進度，並不因登記 task_ref 而變成受管執行回合。
此入口適用於能提供程式成果及 Node 測試的工程工作。一般問答、非受管回合、
其他主機或未登記專案，不能聲稱已有自動診斷。已受管的執行者不得再次派送自己。

候選通知附上同一安裝版本的 `life-harness-client.mjs prepare` 精確命令。
Codex 執行後取得 `command` 與 `readbackCommand`：先在獨立原生工具呼叫原樣執行
`command` 產生收據，再於另一呼叫執行 `readbackCommand` 核對既有 `/diagnostic`。
prepare 只在本機系統暫存區的新 `lattice-diagnostic-*` 目錄保存來源及準備產物，不放進 Git 工作目錄、不執行
回傳命令、不延長期限、不覆寫舊證據。多失敗選擇依據保留在候選紀錄中，原始事件不刪除。

候選發現、送出提示、診斷讀回與工作完成是不同狀態；送出提示意圖不代表 Codex
已採用或診斷成功。失敗、過期、身分漂移與許可拒絕保留原有守衛，不自動重送。
新接線的實際涵蓋範圍以該版本測試與真實受管任務的讀回結果為準。

### 從目前正式回合取得診斷來源

`GET /api/formal-work/<taskRef>/diagnosticsource` 接受 `projectId`、`claimId`、`threadId`、
`turnId`，以及可選的 `failureItemId`。省略 failure ID 時只選目前回合的唯一失敗命令；
多筆回 409 `CONTROL_DIAGNOSTIC_SOURCE_AMBIGUOUS_FAILURE`，附至多 16 個同回合 ID，
須明示選定後重查，不搜尋歷史回合。其餘欄位／重複欄位拒絕。

服務只對自己目前持有的活動回合讀取兩次 Runtime／原生 thread，核對同連線保存的
`item/completed` 完整事件；待決許可、拒絕、熔斷、封存、完成、不明身分或讀取期間漂移
均拒絕。回應只含該失敗 item 的 `failureJson`、其 UTF-8 SHA-256、工作目錄、binding 與
context，不含整段對話。JSON 是原生 item 完整欄位的序列化，並非原始傳輸封包 bytes。
失敗 JSON 必須小於 1 MiB，仍沿用 prepare 的命令／輸出大小限制。
有效期固定為通知觀察時間加五分鐘，重查不延長；查詢五秒包含排隊與來源讀取。
逾時會結束這次查詢並忽略晚到結果，但不取消底層 Runtime 請求或清空其共享佇列。

`context.authority="authorized"` 只投影既有正式執行範圍，以相容 prepare 格式；
`contextBasis="existing_owned_formal_execution"`、`grantsNewAuthority=false`、
`repairAuthorized=false`、`advisoryOnly=true`、`adopted=false` 明確保留邊界。
`nativeSourceVerified=true` 僅證明當次本機來源核對；既有 CLI 收據的 provenance／authorization
旗標仍維持 false，來源入口不授權修復、不讀寫檔案、不執行命令、不啟新回合或呼叫模型。

在已明示授權的正式工作中，Codex 可按下列順序操作；`$projectId`／`$taskRef` 必須取自
既有正式工作，`$importer` 是已明示選定的實際 importer 絕對路徑，不能編造身分或 context。
`$deliveryRoot` 必須是持有此回合的 Control server 所使用的同一份交付根；prepare CLI、
diagnostic CLI 與 server verifier 不可混用不同 checkout，也不注入歷史 helper 特例。

```powershell
$base = 'http://127.0.0.1:4317'
$detail = Invoke-RestMethod "$base/api/formal-work/$taskRef`?projectId=$projectId"
$claims = @($detail.claims | Where-Object { $_.phase -eq 'EXECUTION' -and $_.turn_status -eq 'TURN_BOUND' })
if ($claims.Count -ne 1) { throw '目前沒有唯一的活動執行回合' }
$claim = $claims[0]
$query = "projectId=$projectId&claimId=$($claim.claim_id)&threadId=$($claim.thread_id)&turnId=$($claim.turn_id)"
$source = Invoke-RestMethod "$base/api/formal-work/$taskRef/diagnosticsource?$query"
# 在此工作已允許寫入的範圍明示建立新封存目錄；已存在即停止，不覆寫。
$archive = Join-Path $source.projectRoot 'diagnostic-source-01'
New-Item -ItemType Directory -Path $archive -ErrorAction Stop | Out-Null
[IO.File]::WriteAllText((Join-Path $archive 'source.json'), ($source | ConvertTo-Json -Depth 12), [Text.UTF8Encoding]::new($false))
$failureFile = Join-Path $archive 'failure.json'
[IO.File]::WriteAllText($failureFile, $source.failureJson, [Text.UTF8Encoding]::new($false))
$inputFile = Join-Path $archive 'prepare-input.json'
$prepareInput = @{ projectRoot=$source.projectRoot; importer=$importer; failureFile=$failureFile;
  failureSha256=$source.failureSha256; outputDirectory=(Join-Path $archive 'receipt');
  mode='receipt'; receiptBinding=$source.binding; context=$source.context }
[IO.File]::WriteAllText($inputFile, ($prepareInput | ConvertTo-Json -Depth 8), [Text.UTF8Encoding]::new($false))
node --experimental-vm-modules --disable-warning=ExperimentalWarning "$deliveryRoot/apps/lattice-control/src/relative-module-prepare-cli.mjs" $inputFile
```

prepare 回覆 `prepared` 後，把回傳的完整 `command` 原文交給同一正式活動回合的獨立
原生 shell 工具呼叫；不使用 `Invoke-Expression`、額外命令或重新拼接。再使用 source 的
binding 查原 `/diagnostic` 入口。獨立 shell 呼叫不保留前次變數，以下 `$archive` 必須設為
前一步實際建立的絕對路徑，再讀回已封存的來源：

```powershell
$base = 'http://127.0.0.1:4317'
$source = Get-Content -Raw -LiteralPath (Join-Path $archive 'source.json') | ConvertFrom-Json
$binding = $source.binding
$taskRef = $binding.taskRef
$receiptQuery = "projectId=$($binding.projectId)&claimId=$($binding.claimId)&threadId=$($binding.threadId)&turnId=$($binding.turnId)&failureItemId=$($binding.failureItemId)"
Invoke-RestMethod "$base/api/formal-work/$taskRef/diagnostic?$receiptQuery"
```

可省略 `diagnosticItemId`：只在同一目前回合、該失敗之後的 completed command 中，
用原有精確命令／binding／效期／結果格式驗證選取唯一收據，並核對兩次原生讀取一致。
無合格項目拒絕；多筆合格收據回 409 `CONTROL_DIAGNOSTIC_AMBIGUOUS_RECEIPT`，
不猜最新、不搜尋歷史回合。若已知此次命令的原生 ID，仍可明示 `diagnosticItemId`；
明示 ID 不符時不會改選其他項目。
精確命令比較與原有信任旗標不變；過期、歧義或來源變動停止，不自動重試或修復。

### 明示不恢復既有工作

```powershell
node apps/lattice-control/src/server.mjs --no-auto-restore
```

此啟動方式同時跳過全專案正式工作 `restore` 與主要對話佇列的 constructor 啟動恢復；
已保存的排隊訊息保留，不因啟動被領取或派送。之後明示的正式工作／主要對話操作仍可用，
既有執行期間的佇列與終態事件處理不變；此旗標不會全面停用佇列或更改排程／全域設定。
`GET /api/state` 的 `formal_startup` 回報 `{mode:"explicit-only",restoreRequested:false}`，
正式工作入口仍可用。省略旗標維持既有兩條自動恢復路徑，回報 `restore-existing`／true；
這是啟動政策，不表示任何工作已成功恢復。直接組裝 server 則回報 `manual-composition`。
此選項已做本機聚焦測試，尚未因本次原始碼提交而部署到已安裝服務。

正式 Control 的 MCP 許可適配僅支援已觀察的 `lattice_task_status` 空表單，且 task_ref
必須是目前工作。依本機 Codex `0.155.0-alpha.16.4` 匯出的 App Server schema，使用
`action: accept|decline|cancel` 回覆。待決定問題沿用工作詳情的 `pending_questions`；
明示回答沿用 `POST /api/formal-work/<task_ref>/answer`，body 為
`{projectId, questionId, action}`。只處理一次許可，不轉送 `_meta` 或 session/always 選項。
必須先保存回答，才對同一 native request／連線／claim／turn／input 回覆；原生傳送結果
不明時不重送批准，新的請求需要新的明示決定。未知形狀與過期回答安全拒絕。
拒絕狀態由 PostgreSQL 完整觀察列依目前 claim／最新 dispatch／turn／input 推導為
`mcp_permission:{version:1,denied:boolean}`，不採信 payload.binding，也不受預覽裁剪影響。
每次送出接受前重新查核，包括已保存但未收到確認的回答；版本缺失或未知一律拒絕。
明示 bootstrap 會驗證完整已知目錄後交易式升級讀取函式，保留原始 v1 SQL 與資料；
未知目錄拒絕升級。隔離 PostgreSQL 已驗升級、回滾、資料／權限保留與第二次執行不變。
上述單元與隔離資料庫證據，與已完成的受控 E2E 分開解讀；一般交付仍需當輪同根接線與
實際 Runtime 讀回驗證。未回答不代表接受，外部 message／metadata 也不能授權。
