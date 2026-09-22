# LATTICE — Codex App 的三核心後台

**讓 Codex 的工作有正式任務、可查證的紀錄，以及程式碼關係圖譜。**

LATTICE 是為 **Codex App** 設計的本機工作後台。你在 Codex 裡提出需求、查看進度與核准操作；
LATTICE 透過 MCP 提供任務與專案查詢，將正式工作、決策和驗證結果保存在 PostgreSQL，
並用 Graphify 分析程式碼關係。重新連線後，可以用正式任務編號讀回既有狀態與證據。

[下載 Windows 安裝包](https://github.com/z72124223/lattice-devos/releases/download/v2.0.1-rc.7/LATTICE-Setup-v2.0.1-rc.7-windows-x64.exe) · [安裝指南](INSTALL_WITH_CODEX.md) · [版本與驗收附件](https://github.com/z72124223/lattice-devos/releases/tag/v2.0.1-rc.7)

> **目前版本（2026-09-22）**：三核心產品為 **2.0**；最新 Windows 完整安裝包為 **v2.0.1-rc.7 候選版**，約 **607 MB**。
> 已通過獨立 Windows Server 2025 安裝驗收；普通 Windows 10／11 首次啟用 WSL、UAC／重開機及 Codex Desktop 登入仍待乾淨真機驗證。

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

</details>

## 授權

目前尚未選定 `LICENSE`。GitHub 公開可見性與 `git clone` 功能不等於開源授權；第三方依賴保留各自的授權與散布文件。
目前提供本機使用方式，沒有公開雲端服務。
