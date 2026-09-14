# LATTICE DevOS

LATTICE 是支援 **Codex App 的三核心後台**。使用者只操作 Codex App；
LATTICE 保存正式工作與驗收紀錄，提供程式關係查詢。
獨立網頁平台、視覺工作樹／圖譜、聊天畫面及 Windows 桌面外殼已移除。

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

## 使用方式

在 Codex App 透過已設定的 LATTICE MCP 連線使用 latticed。
新工作先呼叫 lattice_runtime_status，再讀取或登記正式工作。
單純讀取狀態不會自行呼叫模型；就緒狀態也不代表任務已驗收完成。

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
