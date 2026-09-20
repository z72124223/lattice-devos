# LATTICE DevOS 2.0

LATTICE 是支援 **Codex App 的三核心後台**。使用者只操作 Codex App；
LATTICE 保存正式工作與驗收紀錄，提供程式關係查詢。
獨立網頁平台、視覺工作樹／圖譜、聊天畫面及 Windows 桌面外殼已移除。

## 讓 Codex 幫你安裝

不熟悉技術設定，可以把下面這段交給自己的 Codex App：

```text
請按照 https://github.com/z72124223/lattice-devos/blob/product/lattice-control-mvp/INSTALL_WITH_CODEX.md
協助我檢查並安裝 LATTICE 與必要依賴，保留我的設定，完成實際驗收。
先確認相容 Graphify 依賴能否取得；缺少時如實回報，不宣稱三核心安裝完成。
```

[完整 Codex 安裝入口](INSTALL_WITH_CODEX.md)包含官方依賴來源、現有安裝命令、
重新連線及驗收要求。目前 v2.0.0 只有 Runtime 下載檔，尚未提供完整已核驗的
Graphify 依賴包；此流程是協助安裝，不是任何電腦皆可完成的一鍵安裝承諾。

## 三核心

| 核心 | 責任 |
|---|---|
| LATTICE 控制 | 正式工作身分、關係、授權、驗收與可重播狀態 |
| PostgreSQL | 唯一權威資料來源，保存工作、決策與證據 |
| Graphify | 可重建的程式關係與影響查詢 |

Hermes 反思功能已自正式產品退休，沒有登入、設定或命令可以重新啟用它。
主版本使用 `LATTICE_RUNTIME_INTEGRATION="GRAPHIFY"`。推理與檢查由 Codex 執行。

工作樹的上下層關係和程式圖譜的呼叫關係仍保存在後台。
移除的是觀看介面，沒有刪除工作資料、驗收結果、程式分析或核心契約。
Codex 負責推理、執行、工作視窗、進度顯示、核准及封存。
LATTICE 不建立第二套通用代理迴圈或操作平台。

## Graphify 何時會執行？

要求 Codex 使用 LATTICE、設定 `GRAPHIFY` 模式，以及每個任務一定使用圖譜，是不同的事。
安裝程式的「全域掛鉤」是寫入 `AGENTS.md` 的工作指示；目前沒有「未查圖譜就拒收任務」的硬性關卡。
`graphify-refresh` 產生或更新圖譜；`lattice_code_relations` 查詢 PostgreSQL 已保存的圖譜，
不會在缺少分析時偷偷重建。任務提交與關係查詢是不同入口，不能只因提交成功就宣稱已使用 Graphify。

`runtime-health`／`receipt-state` 的結果不能證明真正任務流程呼叫過或跳過 Graphify。
已查核版本的關係查詢入口沒有 `CORE_ONLY` 模式攔截，也尚未找到「CORE_ONLY 下執行真正任務、
以 Graphify 函式呼叫計數證明零次執行」的對應測試。
固定版本、原碼行號、現有測試範圍與缺口見 [Graphify 執行方式與證據](docs/graphify-execution-evidence.md)。

## 使用方式

2.0 為三核心正式版，移除舊版 Hermes 啟動命令與四核心模式，屬不相容變更。
目前支援已配置依賴的 Windows 本機環境；新機安裝與備份／還原步驟見
[本機 Runtime 安裝說明](docs/customer-runtime.md)。完整依賴的一鍵可攜封裝仍是候選功能，
不包含在本次正式版保證內。

在 Codex App 透過已設定的 LATTICE MCP 連線使用 latticed。
新工作先呼叫 lattice_runtime_status，再讀取或登記正式工作。
單純讀取狀態不會自行呼叫模型；就緒狀態也不代表任務已驗收完成。

Codex 執行工作後，可用 `--local-result-import` 提交可驗證的本機交付：
Runtime 實際執行指定的 Node 測試、保存證據，再由正式工作帳本記錄完成。
重新連線後用 `lattice_task_status` 讀回結果。此流程不會另啟模型；
舊的受控模型示範流程及歷史失敗收據不能替代這項交付驗收。

升級前保留設定與資料庫備份。移除 LATTICE 設定中的 `LATTICE_HERMES_*` 欄位，
使用 `GRAPHIFY` 模式，並明確設定 `LATTICE_DELIVERY_LAUNCHER`；
更新執行檔後讓 Codex 重新連線，既有程序不會自動換成新版本。

既有本機相容 API 仍供後台工具使用，可用以下命令啟動：

```powershell
npm.cmd ci --ignore-scripts
npm.cmd run backend:start
```

只監聽 127.0.0.1:4317，不提供網頁。舊畫面網址回傳
LATTICE_VISUAL_PLATFORM_REMOVED，提示從 Codex App 操作。
啟動後台不會預先建立另一個聊天連線；已有工作必要的恢復流程保留。

本機專案目錄和非權威安裝觀察保存在
%LOCALAPPDATA%\LATTICE\control\lattice-control.db。
這個 SQLite 檔案不是 PostgreSQL 正式工作的替代品，移除介面時仍保留。

既有專案登記與讀回命令保留：

```powershell
npm.cmd run control:project -- register --name "My Project" --path "C:\absolute\project"
npm.cmd run control:project -- read --project-name "My Project"
```

目錄登記只保存本機 locator；正式身分仍須由 Runtime 的 PostgreSQL
Project Registry 綁定。既有 Codex MCP 設定與憑證不隨介面移除而更改。

## 驗證與限制

Node.js 需要 24.15 或更新版本；Runtime 使用儲存庫固定的 Rust 工具鏈和 PostgreSQL。

```powershell
npm.cmd run control:test
npm.cmd test
npm.cmd run check
```

PostgreSQL 故障時，正式工作不可用。Graphify 可以獨立降級及修復，不能拿其
失敗或「就緒」狀態改寫正式工作結果。
狀態查詢中的 PREPARED 只代表配置存在；Graphify 的完整身分檢查仍在
實際分析時執行。測試通過只證明受測路徑；GitHub 推送、合併及發布另依使用者
當次指示處理。

現行產品方向以 [AGENTS.md](AGENTS.md) 為準；歷史文件與提交保留作追溯，
其中的網頁、桌面安裝及截圖步驟不再是目前產品要求。

目前沒有公開雲端服務。目前尚未選定 `LICENSE`；GitHub 的公開可見性與 `git clone` 功能不等於開源授權。
