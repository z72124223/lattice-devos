# LATTICE DevOS

[![verify](https://github.com/z72124223/lattice-devos/actions/workflows/ci.yml/badge.svg?branch=product%2Flattice-control-mvp)](https://github.com/z72124223/lattice-devos/actions/workflows/ci.yml)

LATTICE DevOS 是一套**本機優先的 AI 開發工作控制台與耐久執行環境**。
它讓 Codex 負責推理、執行與工作視窗，LATTICE 負責保存工作身分、優先度、
驗證狀態與可重播的 PostgreSQL 證據。

> 目前是 Windows 本機 MVP，沒有公開雲端服務，也沒有正式安裝包或 Release。

## 它解決什麼問題

長時間使用 Codex 開發時，最容易失去的是「這個工作是誰、做到哪裡、下一步是什麼，
以及重開後能不能安全接續」。LATTICE 把這些狀態從聊天內容中分離出來：

- **Control 工作台**：建立專案與工作、設定優先度、建立或續接同一個 Codex 工作視窗。
- **耐久 Runtime**：以 PostgreSQL 保存可驗證的工作與唯一工頭 checkpoint。
- **完成流程**：分開呈現執行結束、使用者驗證、安裝觀察與封存狀態。
- **可選分析模組**：Graphify 提供可重建的關係記憶；Hermes 提供反思與建議。
- **安全邊界**：Codex App 繼續擁有 agent loop、context、sandbox、工具、MCP 與核准流程，
  LATTICE 不重新實作第二套代理迴圈。

## 與目前 Codex 平台的分工

依 2026-09-05 查閱的官方文件對照原定功能：

| 原定需求 | 現在使用的能力 | LATTICE 保留的責任 |
|---|---|---|
| 代理執行、對話與上下文接續 | [Codex App Server](https://learn.chatgpt.com/docs/app-server)／[SDK](https://learn.chatgpt.com/docs/codex-sdk) | 耐久工作身分、原 Codex thread 連結、結果與證據 |
| 平行工作與工具擴充 | [Codex 子代理](https://learn.chatgpt.com/docs/agent-configuration/subagents)、App 原生 skills／plugins／MCP | 有界任務與授權資料；不再建通用代理或 MCP host |
| 定時工作與續看 | [平台排程](https://learn.chatgpt.com/docs/automations?surface=app) | 保存任務結果；不另造排程器，本輪既有排程維持暫停 |
| 長期記憶與關係查詢 | Codex 管理工作上下文 | PostgreSQL 保存權威事實；Graphify 衍生查詢，Hermes 建議不直接改寫事實 |
| 工程進度與交付 | Git、GitHub PR／CI | 可定位的本機提交與交付證據；已淘汰重複的本機工程看板／交付 finisher |

目前新 Control 工作與主對話預設 `gpt-6-astra`；既有 Codex thread 續接時保留原模型。
推理強度由連線主機的 `model/list` 能力決定，明確指定的強度必須在其支援清單內，
不把 [API 的 Astra 推理選項](https://developers.openai.com/api/docs/models/gpt-6-astra)
直接當成 App Server 的可用性證明。缺少模型或強度時在建立 thread 前停止，不暗換模型。

這項變更沒有遷移既有 managed Foreman／semantic reviewer／Hermes 固定模型路徑：
它們仍有資料庫約束、舊收據及恢復程序依賴，保留相容性，不作為新工程工作的模型政策。
現有 Control 對話介面與受管執行隔離也仍有實際使用，保留用途；不再擴充為第二套 Codex。
歷史 charter、module 規格、plans、tickets、reviews 僅供追溯；現行要求以
[`AGENTS.md`](AGENTS.md) 與其工程契約為準。

## 目前可用

| 能力 | 狀態 | 說明 |
|---|---|---|
| 本機 Control 網頁 | 可用 | 專案、工作、優先度、Codex 工作視窗續接、驗證與封存 |
| Control 重開恢復 | 已驗證 | Control 本機專案目錄 locator、Git／規則觀察、工作、視窗對應與安裝觀察收據可由 SQLite 恢復 |
| PostgreSQL 耐久工作狀態 | 已驗證 | Runtime 使用 PostgreSQL 作為權威事實與收據來源 |
| 唯一工頭 checkpoint | 已驗證 | active／blocked／completed／next action 可在新程序重播 |
| GitHub CI | 啟用 | PR 與正式產品分支執行 Node 專案驗證 |
| 公開雲端服務 | 未提供 | 目前只支援本機執行 |
| 正式安裝包／Release | 未提供 | 目前從原始碼啟動與建置 |

## 三分鐘啟動 Control

### 需求

- Windows 11
- Node.js 24.15 或更新版本
- Git
- 要建立新工作，連接的 Codex App Server 必須在 `model/list` 宣告 `gpt-6-astra`
  與有效推理選項；Codex App、npm CLI 和 API 的模型可用性可能不同。

### 執行

```powershell
git clone https://github.com/z72124223/lattice-devos.git
cd lattice-devos
npm.cmd ci --ignore-scripts
npm.cmd run control:start
```

開啟 [http://127.0.0.1:4317/](http://127.0.0.1:4317/)。

Control 資料預設保存在：

```text
%LOCALAPPDATA%\LATTICE\control\lattice-control.db
```

這個快速開始只啟動 Control 網頁，不會自行安裝 PostgreSQL、啟動完整 Runtime、
呼叫模型或重開 Codex App。Codex 只在使用者按下「開始」或「續接」時連線。

### 登記與刷新本機專案

Control 啟動後，可用同一個通用 CLI 登記其他專案、刷新 Git／規則觀察，或讀回
已保存資料：

```powershell
npm.cmd run control:project -- register --name "My Project" --path "C:\absolute\project"
npm.cmd run control:project -- refresh --project-name "My Project"
npm.cmd run control:project -- read --project-name "My Project"
```

登記保存的是 `CONTROL_LOCAL_CATALOG` 目錄項目：穩定的 Control project ID、經觀察的
本機 Windows 路徑 locator，以及最近確認的 repository 根目錄。它固定回報
`registry_authority: NONE`，不是 Rust Project Registry 身分，不能供 Policy、approval、
lease 或 Runtime authority 使用，也不會產生 `ProjectAuthorityReceipt` 或 PostgreSQL
persistence receipt。
Catalog 可保留舊式顯示名稱，但只有 NFC、最多 64 個 Unicode 字元／256 UTF-8 bytes、
且不含可辨識秘密形狀的名稱能成為正式 task data；不相容的被選專案會回傳
`REGISTERED_PROJECT_NAME_UNSUPPORTED`。秘密形狀的 project ID 也不能進入 Registry／Task Ledger
綁定，會回傳 `REGISTERED_PROJECT_ID_UNSUPPORTED` 或在 MCP 邊界直接拒絕。這些舊資料
不是正式綁定候選，因此不會毒化已登記候選的精確 ID、唯一名稱或無 selector
唯一性判定；若明確用 ID 選到 legacy row，則會回傳可修復的未登記錯誤。

Git branch、HEAD、dirty、remote、upstream/ahead/behind 和規則文件 SHA-256 都是帶觀察
時間、可重跑且只保留最新一次已完成檢查（可能是 partial）的 observation；Control 不保存規則文件全文，
remote URL 會先移除 credential。掃描只接受本機 drive-letter 路徑，不跟隨 symlink／
junction、`.git` metadata redirect 或 repository-local config include，並有檔案、總位元組、
文件數與時間上限。這代表 linked worktree 目前會得到可解釋的 partial Git observation，
而不是跨出登記路徑讀取外部 metadata。加上 `--json` 可取得帶固定 schema version 的
機器可讀輸出。

Runtime MCP 的 `lattice_task_submit` 現在可把自然語言 objective 綁到這個 Catalog 中
唯一、可重讀的已登記 locator，再由既有 PostgreSQL Project Registry 取得正式權威並
在 Task Ledger 建立 `GENERAL_TASK_INTAKE`／`DRAFT` 任務；`CONTROLLED_CODEX_CANARY`
仍保留相容。一般任務提交只建檔，不會啟動 Agent、建立規格／tickets，或繞過付款、
外部動作、merge、deploy 等授權。Control schema 會在任何寫入前拒絕較新或漂移的
資料庫。舊 binary 不支援直接開啟 v1 資料庫；若要
downgrade，應還原 migration 前備份。舊程式留下的半登記 row 會顯示為
`LEGACY_CONTROL_PROJECT`，重新登記同一路徑後才會被採用為 catalog locator。

## 架構

```text
瀏覽器
  │
  ▼
LATTICE Control ── SQLite（本機 UI 與非權威安裝觀察）
  │
  ▼
Codex App ── MCP ── latticed Runtime ── PostgreSQL（耐久事實與收據）
                                  ├── Graphify（可重建的關係記憶）
                                  └── Hermes（反思與建議）
```

資料權責刻意分開：

- PostgreSQL 是 Runtime 工作與收據的權威來源。
- Control 的 SQLite 保存本機介面狀態，不取代 PostgreSQL 的 Runtime 真相。
- GitHub 保存正式產品程式、PR 與 CI 交付證據。
- 安裝觀察只證明當時看到的檔案與 SHA-256，不等於持續健康或正式發布。

## 驗證

```powershell
npm.cmd run check
npm.cmd run control:test
npm.cmd test
npm.cmd run verify
```

完整 Runtime 另需要 Rust 工具鏈與 PostgreSQL 17。Runtime 元件可分別驗證；
Graphify 或 Hermes 降級時，不得偽造 PostgreSQL 工作狀態或交付收據。

## 儲存庫導覽

- [`apps/`](apps/)：Control 與 Runtime 應用程式。
- [`crates/`](crates/)：Rust 領域、持久化與整合元件。
- [`db/`](db/)：PostgreSQL migration 與 extension。
- [`docs/`](docs/)：架構決策、規格與可追溯工程證據。
- [`schemas/`](schemas/)：跨元件資料格式。
- [`AGENTS.md`](AGENTS.md)：目前的產品方向與工程邊界。
- [`PLANS.md`](PLANS.md)：目前產品計畫；沒有進行中工作時會明確標示。

## 目前限制

- 主要實機驗證環境是 Windows；CI 另在 Linux 執行 Node 驗證。
- 尚未提供一鍵安裝、雲端部署或一般使用者帳戶系統。
- GitHub 的公開可見性與 `git clone` 功能不等於開源授權。目前尚未選定 `LICENSE`；
  專案沒有授予一般重用、修改或散布授權。
- 高風險外部操作、付款、帳戶與憑證變更、公開網路暴露與不可逆刪除仍需要明確授權。

## 專案狀態來源

- 目前產品：GitHub 預設分支 `product/lattice-control-mvp`。
- 本機工作真相：LATTICE／PostgreSQL。
- 程式交付真相：GitHub 提交、PR 與 CI。
- 根目錄 `HANDOFF.md` 只表示是否存在尚待接手的工作，不再保存歷史施工日誌。
