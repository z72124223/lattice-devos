# 產品計畫

## 已交付基線

- 唯一工頭在工程中發現缺少依賴時，可把父工作耐久記為
  `BLOCKED`，並保存 `depends_on` 與明確 next action。
- 子工作綁定封閉的 task、branch、worktree 與 base SHA；呼叫者不能注入
  任意路徑、分支、hook、filter 或 merge driver。
- 依賴只有在 live Git 證明已安全整合後，才能透過後續 `ACTIVE`
  checkpoint 解除阻塞；衝突、漂移與不確定狀態一律 fail closed。
- PostgreSQL fresh-process replay 可還原父／子身分、分支／工作樹、基準
  SHA、阻塞／續接狀態與下一個動作。
- Codex App 不需重開；版本化 Runtime artifact 可由 fresh Codex app-server
  reload，並從既有 PostgreSQL 恢復同一份耐久事實。

## 穩定邊界

- 沿用現有 Foreman snapshot 與 Task Ledger／PostgreSQL schema-v6，不新增資料表或第二份真相。
- `blocker_ref` 新增封閉、可版本化的 dependency binding；舊字串 blocker 仍可 replay。
- 子工作樹由既有 `GitWorkspace` 安全建立；MCP 不接受任意路徑或任意分支。
- 新 `BLOCKED` 寫入及 `BLOCKED -> ACTIVE` 續接前，都要驗證工作樹身分、乾淨度、base/HEAD 祖先關係與已整合證據；不確定或衝突一律保留 `BLOCKED`。
- 不修改 Task Domain 的 terminal `Blocked` 語意；這是唯一工頭協調 snapshot 的 overlay。
- 不碰、移除或整理任何既有髒工作樹，不使用 reset、clean、force push，也不重開 Codex App。

## 目前方向

- 正式產品分支是 `product/lattice-control-mvp`。
- 目前沒有另一項已核准的產品實作；下一個工作須重新做 live audit、建立
  耐久 task 身分與有界工作樹。
- 易變 SHA、PR、CI、安裝收據與 replay 歷史保留在 Git 與 LATTICE；完整
  驗收索引由 TASK-106 workflow ledger 提供，不複製成會過期的計畫狀態。

## 尚待產品擁有者決定

- 專案尚未選定開源授權；本次不改公開可見性。
