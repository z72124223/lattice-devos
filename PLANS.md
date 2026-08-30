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

- 沿用現有 Foreman snapshot 與 Task Ledger／PostgreSQL schema-v7；一般任務只在正式
  Task Ledger owner 內新增 ingress claim、submission envelope 與 create-only stream，
  不建立 Control 影子任務或第二份狀態機。
- `blocker_ref` 新增封閉、可版本化的 dependency binding；舊字串 blocker 仍可 replay。
- 子工作樹由既有 `GitWorkspace` 安全建立；MCP 不接受任意路徑或任意分支。
- 新 `BLOCKED` 寫入及 `BLOCKED -> ACTIVE` 續接前，都要驗證工作樹身分、乾淨度、base/HEAD 祖先關係與已整合證據；不確定或衝突一律保留 `BLOCKED`。
- 不修改 Task Domain 的 terminal `Blocked` 語意；這是唯一工頭協調 snapshot 的 overlay。
- 不碰、移除或整理任何既有髒工作樹，不使用 reset、clean、force push，也不重開 Codex App。

## 目前方向

- 正式產品分支是 `product/lattice-control-mvp`。
- 已核准的 `apps/lattice-control` Codex App Server 生命週期修復已完成本機
  實作與真實驗收；保留在產品分支的本機提交，不推送、合併或部署。
- 第三階段讓 `lattice_task_submit` 接受自然語言 objective，透過 Control locator 與
  PostgreSQL Project Registry 的明確橋接建立可由 `task_ref` 重啟查回的正式 `DRAFT`
  任務；既有 canary 相容性保留，任務建檔與 Agent 執行／規格／tickets／部署分離。
- 第四階段依 `SPEC-011`／`ADR-028` 接上正式工頭：create-only intake 只可唯一連到
  同一 Task Ledger 的 TaskSpec successor；取得 task/spec/budget-bound execution authority
  後才原子 claim、啟動 exact Codex turn、監看／有限修復、獨立驗證並將證據保存到
  PostgreSQL Task Ledger／Artifact Store。程式任務驗證成功停在 merge 關卡。
- 2026-08-30 的 fresh WSL2 window 已完成官方隔離登入與 zero-model preflight，
  並執行該窗口唯一一次 full live run。task `db30e0c8...` 在 provider dispatch 前
  因 ACTIVE status RPC 逾時失敗；approval 早已 durable 約 115.6 秒，因此不是舊的
  promotion／approval 競態。durable provider effect、attempt、model call、thread、
  turn、outbox 均為 0，實際 token／cost 為 null／未知。產品 status 內層卡點缺少
  telemetry，不能宣稱已定位或已修好。
- Harness 已把 ACTIVE 外層窗口綁到產品 stage contract（目前預設 480 秒），所有
  status RPC 使用各 stage 剩餘時間；逾時後 session 立即標成 contaminated 且禁止
  再呼叫。delayed／hung／late-response 行為測試與 create-new、digest-bound failure
  receipt 行為測試均 PASS。舊 `db30e0c8...` evidence 不回寫 receipt；新持久化只
  適用未來執行。
- 本窗口唯一真實 attempt 已耗用，Phase 4 維持 `NEEDS_REVIEW`。restart／reconnect／
  outbox 與 final full gates 全部 `NOT RUN`，本機 commit 未建立；任何新真實驗收
  都需要新的明確授權與全新 task／thread／turn／attempt identity。
- 易變 SHA、PR、CI、安裝收據與 replay 歷史保留在 Git 與 LATTICE；完整
  驗收索引由 TASK-106 workflow ledger 提供，不複製成會過期的計畫狀態。

## Codex App Server 生命週期修復

### 目標

讓並行 connect、turn readiness、中斷、一次有界 retry、fresh-process
read/resume/reconcile 與主動 request 回覆都能 fail closed，且只有關聯的
`turn/started` 才把 LATTICE work item 標成 `running`。

### 全域策略

以目前 Control 實際使用的 `codex-cli 0.144.6` 生成 Schema 為協定基準；先用
既有 Node 測試鎖住競態，再在同一產品路徑做最薄狀態機與原子 claim，最後用
兩個真實 Codex thread 完成 A-F 有界驗收。

### 非目標

- 不導入 Temporal、LangGraph、OpenHands、Process Compose、新 UI 或第二份任務真實。
- 不接線 Rust delivery adapter 或 Hermes broker，也不重造 Codex agent loop。
- 不推送、合併、部署、發布或改公開可見性。

### 已確認事實

- 基線提交只保存兩份既有失敗證據，開始修改前工作樹沒有其他未提交變更。
- 兩次真實失敗都沒有收到 `turn/started`；第二次 interrupt 回覆 `no active turn to interrupt`，fresh process 讀取又遇到 not-loaded／empty-rollout。
- 官方與本機 Schema 都區分 `turn/start` RPC 回覆、`turn/started` 與 `turn/completed`。

### 已驗證結果

- 同一個已初始化的 App Server 程序承載兩個獨立真實 thread；兩者都收到
  可關聯的 `turn/started`，且在其中一個完成前已同時 active。
- active turn 的 interrupt 收到精確 `interrupted` 終態；一次有界 retry 完成，
  已完成工作沒有新增 turn。
- 關閉並重建 Control store、service 與 App Server 後，兩個既有 thread 均由
  已保存 ID resume/read/reconcile，沒有重做 completed turn。

### 實作步驟

- [x] 以聚焦測試重現 single-flight、timeout/cleanup、readiness、interrupt、reconnect 與防重複缺口。
- [x] 實作接頭生命週期與服務／store 的原子、可關聯狀態轉移。
- [x] 跑聚焦與完整 Node 驗證，完成獨立程式碼及架構審查並修正 P0/P1。
- [x] 跑真實 App Server A-F 驗收並保存 PASS 原始證據；本機提交為最後一步。

### 驗證與風險

- 聚焦：`node --test apps/lattice-control/test/codex-app-server.test.mjs apps/lattice-control/test/control-plane.test.mjs`。
- 完整：`npm.cmd run verify`、`git diff --check` 與真實 A-F runner。
- 結果：Control 42/42；全庫 117 PASS、0 FAIL、1 個既有不適用 skip；
  A-F 全部 PASS；獨立 code/architecture review 為 GO，P0/P1 皆為 0。
- 最高風險是 rollout 尚未落盤時誤報可恢復；任何 read/turn 關聯不完整都停止於 fail closed。

### 漂移紀錄

- 本機 `0.144.6` 生成 Schema 與真實事件確認原路線；沒有導入新框架、UI、
  agent loop 或第二份任務真實。

## 尚待產品擁有者決定

- 專案尚未選定開源授權；本次不改公開可見性。
