# 產品計畫：TASK-106 依賴暫停與重開續接

## 目標

讓唯一工頭在工程中發現缺少依賴時，可以把父工作安全記為 `BLOCKED`，建立並綁定一個有界依賴分支／工作樹；依賴完成且已安全整合後，才把父工作恢復為 `ACTIVE`。PostgreSQL 的 fresh-process replay 必須還原父／子任務、父／子分支與工作樹、基準 SHA、狀態及下一個動作。

## 策略與邊界

- 沿用現有 Foreman snapshot 與 Task Ledger／PostgreSQL schema-v6，不新增資料表或第二份真相。
- `blocker_ref` 新增封閉、可版本化的 dependency binding；舊字串 blocker 仍可 replay。
- 子工作樹由既有 `GitWorkspace` 安全建立；MCP 不接受任意路徑或任意分支。
- 新 `BLOCKED` 寫入及 `BLOCKED -> ACTIVE` 續接前，都要驗證工作樹身分、乾淨度、base/HEAD 祖先關係與已整合證據；不確定或衝突一律保留 `BLOCKED`。
- 不修改 Task Domain 的 terminal `Blocked` 語意；這是唯一工頭協調 snapshot 的 overlay。
- 不碰、移除或整理任何既有髒工作樹，不使用 reset、clean、force push，也不重開 Codex App。

## 已確認事實

- 正式產品分支是 `product/lattice-control-mvp`。
- 正式基準在開工時已與遠端產品分支核對一致；易變 SHA 只保留在 Git 與耐久驗收證據，不寫進公開計畫。
- 現有 snapshot 已保存父分支、父工作樹、父 HEAD、state、blocker 與 generation。
- 現有 `GitWorkspace` 已提供安全建立受控工作樹與 fail-closed integration probe。
- 現有 `BLOCKED` 沒有結構化依賴身分，也沒有解除阻塞前的整合證明。

## 執行步驟

1. 固定 SPEC-010、TASK-106 與模組契約。（目前）
2. 先寫純狀態、MCP、Git guard 與 fresh-process replay 的失敗測試。
3. 實作最小結構化 binding、受控工作樹入口、Runtime projection 與解除阻塞 guard。
4. 跑 focused/full tests、格式／lint、獨立 code/architecture review。
5. 乾淨提交、非強制推送、遠端 SHA、PR/CI、正式分支合併。
6. 不重開 App 的部署／安裝、receipt、即時 Runtime 與新程序 replay 重驗。

## 完成條件

只有上述六步全部成功，且 Git、GitHub、部署收據、即時 Runtime 與 fresh-process PostgreSQL replay 都有當前證據，才可結束本計畫或 Goal。任何單一關卡失敗都停止後續交付並保留可續接證據。

完成後，公開計畫只保留穩定產品方向；執行歷史保留在 Git 與 LATTICE。

## 尚待產品擁有者決定

- 專案尚未選定開源授權；本次不改公開可見性。
