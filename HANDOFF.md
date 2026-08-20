# TASK-089 追溯對象治理修復交接 — 2026-08-21

## 狀態

`NEEDS_REVIEW`：已完成 fail-closed `evidence_subjects` 與獨立審查修復；等待
本次乾淨提交後執行 `npm.cmd run delivery:finish` 的非強制推送、精確 remote SHA
對帳與共用工程頁刷新。未合併、部署、發布或封存。

## 本次完成範圍

- 新欄位只保存 TASK-050／TASK-075 的追溯 provenance；`depends_on` 仍是唯一的
  成功終態交付門檻。非終態 subject 不會阻擋引用它的修復票，也不會被標成完成。
- finisher 和 dashboard 從 captured HEAD 共同拒絕缺失、重複、非法、重疊、自指與
  provenance cycle；dashboard 輸出 `ticket.evidenceSubjects` 作為唯讀證據。
- 第一次 finisher 以 `DEPENDENCY_INVALID` 保守停止：遞迴 subject 檢查錯誤解析
  TASK-050 歷史 multi-line dependency。已修正為只走 provenance edges，並新增
  legacy dependency regression；修復後完整回歸已通過，尚待提交後重跑交付。
- Protocol 1.2、SPEC-005 v7、module constitution 1.6 與 project validator 均已
  同步；移除 protocol 規則會被治理測試拒絕。

## 已驗證

- focused finisher／dashboard RED→GREEN；governance 33/33 PASS。
- 修復後 `npm.cmd run verify` 139/139 PASS；`git diff --check` PASS。
- 獨立 read-only code/security 與 architecture review：No findings。
- 未執行 PR、合併、部署、發布或封存。

## 後續

1. 重看最終 diff 與乾淨狀態，建立修復 commit。
2. 重跑 `npm.cmd run delivery:finish`。
3. 確認 local SHA、`ls-remote`、dashboard `generatedAt`／branch／HEAD／terminal
   status，回報工頭；本 worker 不自行封存。

---

# TASK-078 看板交付就緒修復交接 — 2026-08-21

## 狀態

`NEEDS_REVIEW`：ISSUE 終端證據投影與 TASK 交付就緒修復已完成、測試通過，
等待本次乾淨提交後執行 finisher 的非強制推送與共用工程頁刷新。未合併、部署、
發布或封存。

## 本次修復

- 看板從各 worktree 的 captured `HEAD` 讀取唯一的 ISSUE-007／ISSUE-008
  終端證據；未提交、GitHub-only、重複、號碼／分支不符或非終態，一律維持
  `UNKNOWN`／partial，不會被派工。
- `outcome` 表示功能成果，新增 `delivery` 表示 finisher 交付就緒性。成功的
  TASK 若已提交的 `depends_on` 不是唯一成功終態，會是 `delivery.state: BLOCKED`
  並停止派工／封存暗示；不再將 TASK-043 類型的完成實作誤當可交付。
- 僅 canonical `feature/task-NNN-*` 或 `feature/issue-NNN-*` 取得 TASK／ISSUE
  identity。`integration/task-041-task-042` 等分支保持一般 `BRANCH`，不會與
  `feature/task-041-rust-ci` 產生重複 TASK identity。

## 驗證與審查

- Dashboard focused：23/23 PASS；包含 Issue 7/8 成功與全部拒絕、依賴阻擋、
  integration identity 回歸。
- Finisher focused：41/41 PASS；governance：21/21 PASS。
- `npm.cmd run check` PASS；`npm.cmd run verify`：123/123 PASS；
  `git diff --check` PASS。
- 程式碼／安全與架構自審：No P0/P1 findings；沒有新增 writer、Git mutation、
  remote authority、服務或資料庫。獨立 reviewer `not proven`。

## 交付與使用

1. 建立此修復的乾淨 commit，執行 `npm.cmd run delivery:finish`。
2. 成功輸出必須有非強制 push、精確 remote equality 與 dashboard refreshed；
   由工頭核驗，不自行封存。
3. Issue 7／8 工作窗口只需保留各自已提交的唯一終端 ISSUE evidence 後重跑
   finisher；不建立或冒用 TASK-007／TASK-008。

---

# TASK-078 相容性修復交接 — 2026-08-21

## 狀態

`NEEDS_REVIEW`（僅等待本次乾淨提交後的 finisher delivery）：TASK-078 已補上
fail-closed 相容性修復，尚未推送、合併、部署或封存。

## 本次完成範圍

- `feature/task-nnn-*`：從 captured commit tree 的唯一 terminal TASK ticket
  授權；branch number 與 ticket ID 必須一致。`PLANS.md` 是共享規劃索引，不是
  平行交付鎖。TASK-042 在 PLANS 指向 TASK-033 時的成功回歸已覆蓋。
- 若 TASK 宣告 `depends_on`，每個依賴必須在同一 captured tree 中唯一存在且為
  successful terminal；不存在或未完成均拒絕。
- `feature/issue-nnn-*`：只接受其自身提交的
  `docs/issues/ISSUE-nnn-*.md` terminal evidence。檔名、`issue_id`、分支號與
  exact branch 必須一致，且 evidence 自帶 remote、identity、push/archive policy。
  GitHub branch metadata、任意 feature branch 與 TASK 編號碰撞都不授權。

## 驗證與審查

- `node --test test/engineering-delivery-finisher.test.js`：聚焦 ISSUE 與平行
  TASK 回歸 PASS。
- `npm.cmd run check`：PASS（`files=522 constitutions=27 tickets=49 current_tasks=1`）。
- `npm.cmd run verify`：PASS，120/120、0 fail；`git diff --check`：PASS。
- 程式碼／安全審查：No findings，P0/P1=0；本窗口自行做只讀審查，獨立性
  `not proven`。
- 架構審查：PASS。ISSUE evidence 是同一 module 讀取的 committed static
  authorization input；沒有新 writer、durable truth、服務、dependency 或 ADR。

## 交付與後續

1. 建立本次乾淨 commit。
2. 在此 TASK-078 worktree 執行 `npm.cmd run delivery:finish`；它只能非強制
   推送本 feature branch、核對 remote SHA，並刷新
   `%LOCALAPPDATA%\LATTICE\engineering-status`。
3. Issue 7／8 原工作窗口各自把 TASK-078 ticket 範例的 ISSUE evidence 加入
   自己分支並提交後，再執行 finisher；不要建立或冒用 `TASK-007`。
4. 不合併預設分支、不部署、不發布、不 force push，且本窗口不自行封存。

---

# TASK-078 一鍵工程收尾交接 — 2026-08-21

## 狀態

`DONE`：已完成「提交後安全推送 GitHub、核對遠端、更新工程地圖，全部成功才封存 Codex 任務」的一鍵收尾工具。這份交接提交後，會由新工具親自完成 TASK-078 的非強制功能分支推送、地圖更新與最終核對；只有看到精確成功標記才封存本 Codex 任務。

## 使用方式

日後每張 TASK 票券明確寫入遠端、GitHub repository 身分、推送政策與封存政策。Codex 完成測試、審查、交接及乾淨提交後執行：

```powershell
npm.cmd run delivery:finish
```

成功時會完成三件事：

1. 只把目前 TASK 功能分支的精確提交非強制推送到票券指定的 GitHub repository。
2. 核對 GitHub 與本機 upstream 完全一致，再安全更新本機工程地圖。
3. 僅在 TASK 成功終止且所有關卡通過時輸出 `LATTICE_DELIVERY_READY_TO_ARCHIVE=1`，接著由 Codex App 原生動作封存當下任務。

任何失敗都會保留任務視窗，方便繼續診斷；不會誤推預設分支、不會 force、不會自動合併、部署或發布。

## 驗證與審查

- 專用測試 35/35、治理測試 21/21、完整專案 114/114 全數通過。
- 獨立程式碼／安全審查：P0=0、P1=0、P2 runtime/security=0。
- 架構審查：PASS，沒有新增產品 runtime、MCP、資料庫或 lease 邊界。
- 精確整合：實作檢查點 `f04b462571e6bdd052db9c4cd343bfc26d158628` 對 GitHub 當時預設 target `8828d2b88faece6b399258744eea4ff8d46f0bea`，零衝突，combined 114/114 PASS，驗證 worktree 已移除。

## 保護邊界

本任務只獲准推送 `feature/task-078-engineering-delivery-finisher`。沒有 PR、預設分支合併、部署、release、公開 hosting、credential 變更、force 或一般資料夾刪除授權。

---

# TASK-077 V3 模型與推理建議交接 — 2026-08-20

## 狀態

`DONE`：使用者要求的模型／推理強度補充已完成，測試與整合模擬均通過。整合仍為 `NEEDS_REVIEW`，因為預設分支沒有機器強制關卡，也沒有合併授權。本交接完成後只會依既定流程提交、非強制推送 TASK-077 功能分支並重新整理本機頁面；不建立 PR、不合併、不部署、不發布。

## 目標與範圍

- 在選取綠色分支並輸入新工作後，用白話推薦目前最省成本但足夠可靠的 Codex 模型與推理強度。
- 把推薦、理由、成本原則與升級條件一併放入可複製的派工文字。
- 保持頁面唯讀；它不會切換模型、建立 TASK／分支、呼叫付費 API 或授權工程動作。

## 使用者現在會看到的建議

| 工作類型 | 建議 | 白話原則 |
| --- | --- | --- |
| 改文字、格式、顏色、重新命名等明確小修改 | `gpt-5.6-luna`＋低推理 | 最快、成本最低；範圍擴大或測試失敗再升級 |
| 一般功能、修正問題、補測試或描述不夠明確 | `gpt-5.6-terra`＋中等推理 | 品質、速度與成本最均衡的預設選擇 |
| 架構、安全、權限、資料遷移、部署或高錯誤代價工作 | `gpt-5.6-sol`＋高推理 | 成本較高，只在風險值得時使用 |

模型依據已標示核對日期 `2026-08-20`。超過 30 天或時間不可信時，頁面會停止指定模型並要求 Codex 重新核對，不會用舊名稱猜測。高風險詞永遠優先於低成本詞。

## 完成內容與檔案

| 路徑 | 用途 |
| --- | --- |
| `scripts/export-lattice-engineering-status.mjs` | 共用、可測試的三層模型建議與 30 天過期保護 |
| `tools/engineering-status-dashboard/index.template.html` | 可見建議卡、三種配置說明及含模型資訊的派工文字 |
| `test/engineering-status-dashboard.test.js` | Terra 預設、Luna 小改、Sol 高風險優先、過期拒絕與頁面輸出回歸 |
| SPEC-004 v3、constitution 1.2、TASK-077、PLANS | 使用者要求、非權威邊界與驗收證據 |
| review／integration／ledger | 審查、精確整合與缺少 GitHub 強制關卡的證據 |

## 驗證與審查

- Dashboard focused：20/20；governance：18/18。
- 最終 feature-source `npm.cmd run verify`：76/76。
- 生成頁面：三模型文字、placeholder 完整替換、可執行 JavaScript 語法與 `git diff --check` 均通過。
- 程式碼／安全審查：先發現 P2 過期模型風險與 P3 提示不明顯，均已修復；最終 P0=P1=P2=P3=0。這次未獲准使用獨立代理，因此 reviewer independence 明確標為 `not proven`。
- 架構評估：沒有 schema、truth、writer、authority、network、dependency、migration 或 hosting 觸發；頁面仍是唯讀展示層。
- 精確整合 checkpoint：`6ca83afc1eae10cba58e5cb49541d0cdd106c584` 對實際預設 target `8828d2b88faece6b399258744eea4ff8d46f0bea`；0 conflict、combined 76/76、cleanup PASS。
- GitHub 即時結果：PR `[]`、CI run `[]`、rulesets `[]`、預設 target 未受保護。技術通過不代表已授權合併。

## 風險與下一步

- 未執行真實瀏覽器點擊／縮放或像素級視覺驗收；生成頁面的結構與 JavaScript 已驗證。
- 關鍵字建議刻意保守並顯示理由；實際工作範圍若不同，派工文字要求先重新評估。
- 接續：提交本交接、跑提交後檢查、非強制推送 feature、核對遠端一致，再重新整理並打開頁面。
- 恢復點：branch `feature/task-077-engineering-status-dashboard`；implementation checkpoint `6ca83af`；先看 `git status --short --branch`。

---

# TASK-077 V2 中文分支工作地圖交接（舊版紀錄）— 2026-08-20

## 狀態

`DONE`：使用者授權的 V2 實作、測試、獨立程式碼／安全審查、架構審查與預設分支整合模擬均已完成。整合狀態維持 `NEEDS_REVIEW`，因為本工作未取得預設分支合併授權，GitHub 也沒有可替代人工決定的強制規則或 CI 關卡。功能分支尚待最後文件提交、非強制推送與推送後重新整理；未建立 PR，未合併、部署、發布或公開託管。

## 目標與範圍

- 目標：把原本偏工程師的卡片頁改成外行人也能閱讀的繁體中文、由上到下、可展開分支樹。
- 使用者可直接看到每個分支的用途、目前狀態與「能不能從這裡安排新工作」。
- 合格節點可選取、輸入工作內容，產生一段可複製給 Codex 的中文派工文字；頁面本身不建立 TASK、不建分支、不寫入 Git 或 LATTICE 權威資料。
- 所有英文分支名稱與 SHA 收進「工程細節（平常不用看）」。
- 不包含 TASK-051 runtime 重跑、PR、預設分支合併、部署、發布、公開託管、憑證變更或破壞性清理。

## 已完成工作

- 匯出器以即時 Git worktree、遠端 heads、ticket 與證據產生 `lattice.engineering-status/2.0` 快照。
- Git commit ancestry 形成穩定的樹；支援沒有本機 worktree 的預設分支、重複 detached 名稱與完整圖形驗證。
- 只有「終態證據完整、工作區乾淨、遠端同步、中文用途已登錄、非 detached/prunable」的節點可派工；任何時間、遠端或 ancestry 證據不可信時皆保守停用。
- 40 個目前節點皆有人工整理的繁體中文名稱與用途；每個新分支也必須在允許路徑內同步更新用途表，專案檢查會阻止遺漏、錯分支或全英文內容。
- 靜態頁支援展開／收合、狀態色、選取派工起點、輸入新工作與複製 Codex 指令；動態資料用安全文字 API 呈現。
- `Open-LATTICE-Engineering-Status.cmd` 會先重新整理再打開 `%LOCALAPPDATA%\LATTICE\engineering-status\index.html`。
- 工程協議 1.0.3 要求交接完成後及每次授權推送後重新整理此工具。

## 主要變更檔案

| 路徑 | 用途 |
| --- | --- |
| `scripts/export-lattice-engineering-status.mjs` | 蒐集即時證據、建立分支樹、保守計算派工資格、驗證後原子輸出 |
| `tools/engineering-status-dashboard/index.template.html` | 繁體中文可展開樹狀介面與可複製派工文字 |
| `tools/engineering-status-dashboard/branch-guide.zh-TW.json` | 每個現有分支的中文名稱與白話用途 |
| `scripts/check-project.mjs` | 強制新分支與用途表、ticket `allowed_paths` 一致 |
| `test/engineering-status-dashboard.test.js`、`test/project-governance-check.test.js` | UI／快照／Git／治理回歸測試 |
| SPEC-004、TASK-077、module constitution、engineering protocol | V2 行為、邊界、驗收與更新規則 |
| review、integration、workflow ledger | 獨立審查與整合證據 |

## 驗證與審查

| 項目 | 結果 |
| --- | --- |
| Dashboard focused tests | PASS，19/19 |
| Governance focused tests | PASS，18/18 |
| 最終提交後 `npm.cmd run verify` | PASS，75/75 |
| 獨立程式碼／安全審查 | PASS，No findings；P0=P1=P2=P3=0 |
| 架構審查 | PASS；無 truth/writer/authority 擴張，無 ADR、migration 或新 dependency |
| 精確預設分支整合模擬 | PASS；`c88cc9293f3c521974afe7abe1f74f9e449cfaa4` 對 `8828d2b88faece6b399258744eea4ff8d46f0bea` 無衝突，combined verify 75/75 |
| 模擬清理 | PASS；臨時 worktree／分支已移除 |
| Live V2 refresh | PASS；40 節點、1 個 root、ancestry available、0 unmapped、3 個當時可派工起點 |

第一次整合模擬在 detached HEAD 專用驗證環境中被分支檢查器保守拒絕；已加入只限「唯讀整合 worktree」及實際 `origin/HEAD` 預設分支的窄化例外，重新獨立審查後於精確 checkpoint 完整通過 75/75。這不放寬一般 TASK worktree 的真實分支檢查。

GitHub 即時檢查顯示此 feature 尚無 PR 與 CI run；repository rulesets 為空，預設分支未受保護。因此「技術上可整合」不等於已取得合併授權，本工作未執行合併。

## 交付收尾與恢復點

- 已驗證實作 checkpoint：`c88cc9293f3c521974afe7abe1f74f9e449cfaa4`。
- 接續動作：提交本交接文件，跑最後完整驗證，非強制推送 `feature/task-077-engineering-status-dashboard`，證明本機／遠端一致，再依協議重新整理並打開頁面。
- 若中斷，從上述 feature worktree 執行 `git status --short --branch`，確認只剩預期文件變更後接續。
- 頁面啟動與 JavaScript 結構已驗證；本次未宣稱做過 screenshot／pixel-level 視覺驗收。

---

# TASK-077 V1 舊版交接紀錄（保留供追溯）

# TASK-077 Local Engineering Status Dashboard Handoff — 2026-08-20

## Status

`DONE` for the authorized local-only TASK-077 implementation, focused/full
verification, independent code/security review, architecture review, and
default-branch merge simulation. Integration is `NEEDS_REVIEW` because no merge
authorization exists and GitHub has no enforcing rule/CI gate for this feature
push. No PR, merge, deployment, release, or public hosting occurred.

## Objective and scope

- Objective: give a non-engineer one double-clickable, static local page that
  refreshes and explains every registered LATTICE worktree/TASK/ISSUE with Git,
  ticket, live remote, PR, and CI evidence.
- In scope: SPEC-004 v1, the new read-only presentation module, exporter,
  launcher, safe static UI, tests, delivery refresh rule, and local generated
  snapshot.
- Out of scope: task/Git/GitHub mutation, LATTICE truth or writer authority,
  TASK-051 runtime rerun, public hosting, PR creation, merge, deployment,
  release, credentials, and destructive cleanup.

## Completed work

- `Open-LATTICE-Engineering-Status.cmd` refreshes first and then opens the exact
  generated page; actual Windows argv, spaces, trailing separator, and launch
  acknowledgement are tested.
- The Node standard-library collector reads 39 registered worktrees, validates
  unique exact TASK tickets, normalizes repository status vocabulary, checks
  live remote heads, enriches one bounded GitHub PR/CI query, and fails closed on
  missing/conflicting/malformed/stale evidence.
- The self-contained HTML shows current focus, plain-language counts, filters,
  search, responsive cards, next actions, and collapsed technical evidence. It
  uses safe text APIs and redacts verified common absolute-path formats.
- Default output is outside Git at
  `%LOCALAPPDATA%\LATTICE\engineering-status\{index.html,status.json}`; refresh
  creates no repository status-update commit.
- Engineering protocol v1.0.2 now requires `npm.cmd run status:refresh` after a
  current handoff and after any authorized push, while explicitly preserving
  the projection's non-authoritative boundary.
- TASK-051 was paused without a runtime rerun. Its live card remains `FAIL`
  while independently showing clean Git, synchronized live remote, PR #13, and
  passing CI.

## Files changed

| Paths | Why | Verification |
| --- | --- | --- |
| `scripts/export-lattice-engineering-status.mjs`, static template, root launcher | read-only collection, safe rendering, one-click local UX | dashboard regressions, live 39-item refresh/open |
| `test/engineering-status-dashboard.test.js` | fail-closed Git/ticket/remote/privacy/Windows coverage | focused dashboard suite passed |
| SPEC-004, module constitution, TASK-077, PLANS | approved scope, ownership, vocabulary, acceptance | `npm.cmd run check` |
| engineering protocol/checker/governance test | mandatory post-handoff/push refresh rule | removal regression and project check |
| TASK-051 ticket | record user priority pause without changing terminal result | live card remains explicit `FAIL` |
| review/ledger/integration artifacts and this handoff | durable evidence and next decision | independent review and combined verification |

## Workflow ledger

| Stage | Status | Evidence / artifact |
| --- | --- | --- |
| Inspect/scope/TDD | PASS | clean source at `6e393f6`; SPEC-004; TASK-077; observable RED then GREEN |
| Focused verification | PASS | dashboard + governance 22/22 after review remediation |
| Full/combined verification | PASS | implementation/default-target combined result 60/60 |
| Code/security review | PASS | independent final P0=0/P1=0/P2=0/P3=0 |
| Architecture review | PASS | no truth/writer conflict, cycle, migration, or ADR change |
| Integration verification | NEEDS_REVIEW | no conflict and 60/60, but no merge authority or GitHub enforcement |

## Verification

- Implementation checkpoint:
  `89de978404acfefcdb0eec23742657636d4cf16d`.
- Combined `npm.cmd run verify`: exit 0, 60/60.
- Combined `npm.cmd run check`: exit 0, 511 files, 26 constitutions, 48
  tickets, one current task.
- Final source-tree `npm.cmd run verify`: exit 0, 60/60; final handoff project
  check: 512 files, 26 constitutions, 48 tickets, one current task.
- `git diff --check`: exit 0 before checkpoint and after final handoff edits.
- Live `npm.cmd run status:open`: exit 0, 39 items,
  `LATTICE_STATUS_OPENED=1`; Git remote and GitHub sources available.
- Browser launch is structurally verified. Pixel-level or screenshot visual QA
  was not requested and is not claimed.

## Review and integration

- Independent code/security and architecture reviews are blocker-free.
- GitHub default target is
  `feature/task-037-full-chain-integration@8828d2b`, not `main`.
- Feature is 500 commits ahead/0 behind that target; detached temporary merge
  was conflict-free and its combined 60-test result passed. Temporary worktree
  cleanup passed.
- GitHub rulesets are empty; target branch protection is absent; workflow CI is
  configured for PRs and pushes to nonexistent `main`, so feature-push CI is
  `missing`, not claimed passing.
- No merge was performed. The separately authorized non-force feature push is
  the remaining terminal delivery command after this handoff commit.

## Risks and open decisions

- Path redaction is heuristic for uncommon mount roots; verified current-machine
  drive, forward-slash drive, UNC, `/home`, `/root`, and `/data` formats pass.
- The 39-item live snapshot is intentionally `partial`: 14 worktrees lack a
  unique exact matching ticket and therefore remain visible as `UNKNOWN` rather
  than being guessed successful.
- The user may later decide whether to integrate TASK-077 into the actual
  default branch. Review, merge, deployment, and release are separate gates.

## Next action

1. Non-force push `feature/task-077-engineering-status-dashboard`, verify remote
   equality, and run the post-push dashboard refresh required by protocol v1.0.2.
2. If desired later, explicitly authorize a separate default-branch integration
   decision; otherwise continue using the local launcher from this feature.

## Restart context

- Current branch/worktree: `feature/task-077-engineering-status-dashboard` /
  `lattice-worktrees/task-077-engineering-status-dashboard`.
- Relevant plan: `PLANS.md`, current TASK-077 completion section.
- First files: `HANDOFF.md`,
  `docs/reviews/INTEGRATION_TASK_077_2026-08-20.md`, and
  `docs/tickets/TASK-077-engineering-status-dashboard.md`.

---

# TASK-050 Autonomy Receipt Closure Handoff - 2026-08-15

## Status

`DONE` for TASK-050 implementation and its required local/runtime verification.
Integration remains `NEEDS_REVIEW` because Draft promotion, human GitHub review,
and merge authorization are absent; no merge was performed.

## Objective And Scope

- Objective: make Task Ledger the sole durable autonomy profile/receipt/hash
  owner and prove exact PostgreSQL restart plus fresh canonical `latticed`
  replay without changing the four-tool/six-field public MCP contract.
- In scope: the 21 TASK-050 allowed implementation/governance/test paths and
  this handoff.
- Out of scope: migration `0006` bytes/catalog, Cargo dependencies, TASK-038,
  product Git effects, credentials, merge, deployment, release, and public
  exposure.

## Completed Work

- Implementation commit `8e5ba40d38b781afff7028841bd981c8dd2b9721`,
  tree `b4478be2801814ffc630cbf113b0a4ffa3a1b591`, is published on
  `feature/task-076-postgres-writer-lease-v2` and attached to Draft PR #12.
- Task Ledger owns the closed profile, typed append plan, canonical hashes, and
  24-scalar replay verifier. Store maps scalars and owns only transaction/I/O;
  Orchestrator supplies a pure recommendation; Ports exposes only legal closed
  lifecycle states.
- ASK_USER performs no Writer/downstream effect. PROCEED binds the exact
  current 15-scalar Writer tuple; stale, wrong-fence, and substituted authority
  fail with zero mutation. Exact retry is replay-derived and read-only.
- The acceptance runner proves initial/restart ASK_USER and PROCEED through four
  fresh canonical `latticed` PIDs, exact startup diagnostics, four tools, six
  public fields, zero prohibited effects, and cleanup.

## Files Changed

| Paths | Why | Verification |
|---|---|---|
| Task Ledger, Ports, Store, Orchestrator, Contracts, Runtime Rust paths | owner-boundary and fail-closed repair | focused suites, strict Clippy, full wrapper |
| TASK-050 scripts | fresh-process PostgreSQL 17 acceptance and self-tests | wrapper exit 0, final PASS marker |
| SPEC-002, Task Ledger/Ports constitutions, TASK-050 ticket, PLANS | approved versioned contract and trace | repository/scope/architecture checks |

## Workflow Ledger

| Stage | Status | Evidence / artifact |
|---|---|---|
| Governance and scope | PASS | SPEC-002 v35; Ledger 2.3; Ports 1.9; Store 1.10; Orchestrator 2.6; latticed 1.8; 21/21 allowed paths |
| TDD and focused verification | PASS | generic-forgery, profile, 24-scalar, authority, retry, Ports, Runtime, Store, and runner RED/GREEN evidence |
| PostgreSQL/fresh process | PASS | run `9f1907f07f5545be9927a8def04529f5`; PostgreSQL 17.10; initial/restart; cleanup complete |
| Code/security review | PASS | independent P0-P3 = 0; `CLEAR_FOR_INTEGRATION` |
| Architecture review | PASS | independent P0-P3 = 0; `CLEAR_FOR_INTEGRATION` |
| GitHub CI | PASS | implementation run `31835654240` / job `94881263124` at `8e5ba40`; closure run `31836370497` / job `94883441544` at `0f4d5c3` |

## Verification

- `powershell.exe -NoProfile -ExecutionPolicy Bypass -File
  scripts/test-task050-autonomy-receipt-acceptance.ps1`: exit `0`,
  `TASK050_AUTONOMY_RECEIPT_ACCEPTANCE=PASS`.
- Receipt raw SHA-256:
  `b2ba6aa3d5f5c6b145de146bbe5b59696322ab3713dabb07a44313f1fcfc2478`;
  final HMAC SHA-256:
  `e953e32d4cc7ab8d9509e18b5bb66c98f449b027d0d25b301dd8038e8da7cc04`;
  10 events, `cleanup_complete=true`, listener and cluster root absent.
- Full wrapper also passed format, changed-slice strict Clippy, affected Rust
  suites, MCP four-tool/six-field regressions, `npm.cmd run check`, and final
  diff checks. Migration/catalog/Cargo frozen paths remain unchanged.

## Review And Integration

- Feature branch is synchronized through GitHub-truth alignment descendants of
  closure head `0f4d5c3`; accepted TASK-050 implementation remains `8e5ba40`,
  and the PR target remains
  `feature/task-075-schema-v5-migration-reconciliation@a3599c1`.
- GitHub reports Draft PR #12 open. The implementation CI is green. Branch
  protection/rules enforcement is not claimed.
- GitHub's default branch remains
  `feature/task-037-full-chain-integration@8828d2b` and trails this combined
  source. Its local branch also has four independent fixes absent from
  `0f4d5c3`, so default movement is blocked pending explicit reconciliation.
- Decision: `NEEDS_REVIEW`. Machine checks pass, but no human review or merge
  authorization exists. No Draft promotion, force push, protected mutation,
  merge, deployment, or release occurred.

## Risks And Open Decisions

- TASK-051 current-machine MCP registration is visible but points to historical
  commit `0b59b79`; it is not verified for accepted TASK-050 implementation
  `8e5ba40`. Semantic invocation remains `UNKNOWN`.
- The dedicated TASK-051 branch is synchronized to the GitHub-truth baseline
  and preserves one untracked runner; its test wrapper is absent. Its ticket
  still names SPEC-003 v4 and latticed 1.7 while current contracts are v5 and
  1.8.
- Any TASK-051 reversible `C:\Users\f7212\.codex\config.toml`
  `[mcp_servers.lattice]` switch must meet that ticket's exact preflight and
  rollback gate; credentials and other tables remain prohibited.

## Next Action

1. Continue TASK-051 on its synchronized dedicated branch: reconcile the
   ticket/contracts, adopt the preserved runner, and complete its wrapper before
   any live acceptance or configuration switch.

## Restart Context

- Current branch/worktree: `feature/task-076-postgres-writer-lease-v2` /
  `lattice-worktrees/task-076-postgres-writer-lease-v2`.
- Relevant plan: `PLANS.md`, section `Current P0 platform live acceptance -
  TASK-051`.
- First file: `docs/tickets/TASK-051-p0-platform-live-acceptance.md`.

---

# TASK-076 Writer Lease V2 Bridge Handoff - 2026-08-14

## Current Outcome

TASK-076 is complete at verified implementation/governance commit
`f32531002a0c6588e96dc9fe0229db7e0ed546e0`, based on
`8b6d5171759e28339d6fe2b66fab0c3b6718c64c`. TASK-075 commit `a3599c1` and
the separately committed migration-byte preservation prerequisite remain
unchanged. The Writer Lease v2 bridge closes the compatibility blocker that
prevented the global-v3 + Codebase-Memory-v2 + Writer-Lease-v1 profile from
advancing to global-v5 + Memory-v3.

The completed ticket adds an append-only Writer Lease v2
bridge while preserving v1 SQL and every semantic row, canonical receipt,
checkpoint, lease revision, and fencing high-water. The fixed transition is
`G3_M2_W1_CURRENT -> G3_M2_W2_BRIDGE -> G5_M2_W2_BRIDGE_PENDING ->
G5_M3_W2_BRIDGE_PENDING -> G5_M3_W2_CURRENT`. Store, Memory, and Writer
administrative runners take global -> Memory -> Writer transaction locks;
only the Writer owner changes Writer identity/ledger, and both pending states
reject runtime use.

The implementation worktree is
`lattice-worktrees/task-076-postgres-writer-lease-v2` on branch
`feature/task-076-postgres-writer-lease-v2`, based on `8b6d517`. The post-fix
official wrapper passed in 368.4 seconds with
`TASK076_WRITER_LEASE_V2_BRIDGE_ACCEPTANCE=PASS`; its embedded TASK-075 and
TASK-050 gates emitted their expected markers. The TASK-050 marker is partial
baseline evidence only: later independent TASK-050 review proved that its
runner did not launch a fresh canonical `latticed` process. Focused checks,
strict Clippy, frozen-history, format, repository, and diff checks pass for
TASK-076. Independent TASK-076 code re-review is clear,
architecture review reports no findings, and integration found zero technical
blockers. GitHub Actions run `31810017320` / job `94798123907` passed on the
exact verified commit.

Integration status is `NEEDS_REVIEW` only because merge/Draft-promotion
authorization and GitHub human approval are absent. No merge or Draft promotion
occurred. TASK-075 has completed its combined-candidate revalidation; TASK-050
is now the sole `in_progress` ticket, and TASK-051 remains blocked. Existing
PostgreSQL listeners at `5432`, `64272`, and `55432`, unrelated dirty
worktrees, and the protected TASK-051 runner must not be touched.

## GitHub Publication Authorization And Safety

On 2026-08-14 the user explicitly authorized routine GitHub publication for
ongoing LATTICE development. GitHub CLI is verified for `github.com` account
`z72124223`, repository `z72124223/lattice-devos`, HTTPS transport, and a
credential stored in the operating-system keyring. Within a bounded authorized
ticket, Codex may create clean commits, push the exact feature branch without
force, and create or update a Draft pull request without repeated approval.

No OAuth token, device code, password, browser/mobile session, or confirmation
value is stored. Merge, Draft promotion, force-push, protected-branch or
repository-permission changes, tag/release, deployment, public exposure,
history deletion, and credential/account/payment mutation remain outside this
authorization.

The first published implementation checkpoint is
`172307c19ee5be3387646858a834b0aa1790d81e`. Its remote head is
`feature/task-076-postgres-writer-lease-v2`; the separately published base is
`feature/task-075-schema-v5-migration-reconciliation` at `a3599c1`. GitHub
Draft PR `https://github.com/z72124223/lattice-devos/pull/12` targets that base
and remains explicitly not authorized for merge or Draft promotion.

# TASK-075 Schema-v5 Migration Reconciliation Handoff - 2026-08-14

## Current Outcome

TASK-075 is complete. Governance selects the exact TASK-022 Project
Registry `0005` source at commit `12f7100` (SHA-256
`b7af1f8a8ac370bbfc8a5312497461587cb8a86eb32ff97e5b865c7ae9bf0dcf`)
as global schema v4 and requires TASK-050 autonomy commit `714f3b9` to be
re-authored as `0006_task_autonomy_receipt.sql`, global schema v5. TASK-022
closure source `a1aced9` remains provenance; none of these commits is treated
as already integrated merely because it is readable.

The fixed compatibility rule is fail-closed: an existing database that placed
autonomy at ordinal `0005` returns
`STORE_MIGRATION_HISTORY_MISMATCH` before migration DDL and is not repaired.
Every Registry command must retain the schema/manifest profile that created its
persistence receipt. Existing v4 commands bind schema `4` and exact manifest
`df3f7ca3687afaa0d1f676158725e6d2f06670e0612df7482aa9d4d244b59f0f`;
new v5 commands bind the exact current v5 profile, and mixed replay must return
the original byte-identical receipt for each command.

The exact base v5 target is 16 `control` tables, 47 retained functions, 19
runtime-executable successors, and 28 historical non-runtime functions.
Codebase Memory v1/v2 bytes and v2/global-v3 receipts remain immutable;
TASK-075 adds only extension v3/global-v5 plus per-analysis profile provenance
for byte-identical mixed graph/reflection replay. Contracts 1.13 and Postgres
Codebase Memory 1.1 own that compatibility amendment.
Project Registry pure semantics stay at 1.2. The TASK-076 wrapper executed all
16 TASK-075 gates, required the exact full-acceptance marker, and closed four
distinct marker-owned PostgreSQL 17.10 receipts for Store-only, catalog,
Memory-v3, and TASK-050 profiles. Code and architecture re-reviews found no
issues, integration found zero technical blockers, and no additional heavy
rerun was needed. The implementation source remains `a3599c1`; the exact
combined revalidation source is `f325310`.

TASK-050 is now the sole `in_progress` ticket. Its original source `714f3b9`
remains provenance only; closure must use the exact combined schema-v5
candidate. Independent review invalidated the prior fresh-process claim and
found unresolved P1 ownership, required-profile, Ports-versioning, and
acceptance-runner gaps. On 2026-08-15 the user approved and consumed the narrow
Human Gate recorded in the TASK-050 ticket: Task Ledger 2.3 owns the versioned
profile discriminator and canonical autonomy receipt/hash semantics; Ports 1.9
carries neutral lifecycle profile evidence; Store 1.10, Orchestrator 2.6, and
`latticed` 1.8 consume that contract; the Contracts public SHA helper is
removed while Contracts remains 1.13 and the six-field MCP wire is frozen.
Governance validation and focused TDD repair are active. TASK-051 remains
blocked until TASK-050 is repaired, freshly revalidated, re-reviewed, and
records its own verified completion. No merge, Draft promotion, deployment,
release, or public exposure is authorized.

# TASK-074 Local Model Assist Handoff — 2026-08-14

## Outcome

The preinstalled Qwen2.5-Coder 7B Q4_K_M model is active through the verified
CUDA llama.cpp runtime on `127.0.0.1:18181`; health and model discovery pass,
and GPU offload is bound to the exact process. A CPU LATTICE test and a local
GPU inference ran concurrently. The Rust test passed, and the model correctly
summarized its output.

The model failed a security-sensitive recovery-classification benchmark by
suggesting a broader retry policy. It is therefore limited to untrusted
summaries, candidate test lists, and mechanical second opinions. Source,
compiler, tests, and the primary agent remain authoritative. No model download,
credential, external API/network, public listener, project write, Git change,
database access, push, merge, or deployment occurred. PID `26732` remains
intentionally running on loopback for bounded assistance.

# TASK-073 Local Handoff — 2026-08-14

## Outcome

Hermes now retires same-process recovery receipts for authoritative initial or
reconciled Failed/Cancelled terminals and for schema-valid immutable completed
outputs that carry the supported Codex app-server failure sentinel. Retired
receipts fail before endpoint I/O; uncertain, malformed, and cross-bound
observations retain recovery state. Commits `1ecdd1d` and `d057ff6` together
close the lifecycle boundary. Adapter all-target verification passed with 85
ordinary tests and 11 explicit live-only ignores, plus 4 preparation tests;
focused regression, format, diff, and independent review gates passed.

# TASK-072 Local Handoff — 2026-08-14

## Outcome

The exact pinned Hermes v2026.8.3 runtime now has a concrete same-port
recovery acceptance through its production WSL/bubblewrap owner. A gated
in-process fake Codex acknowledges one turn, withholds its terminal, and makes
the first observation return exact `HERMES_RUN_DEADLINE_EXCEEDED` with a known
run receipt. Releasing that terminal lets one reconciliation return strict
canonical reflection plus `RuntimeKind::Live` evidence without another
`turn/start`.

The first GREEN exposed a teardown bug: after the authenticated close and
terminal barrier, outer EOF was still classified as failure. `7e83d55` accepts
EOF only after authenticated open, clean terminal, exactly one turn, and
adapter success. The outer WSL relay now ignores only broken-pipe or
connection-reset delivery of an already-received inner response; deterministic
Python tests prove all other socket errors remain fail closed.

The exact recovery, existing success, and existing definitive-failure
no-model live tests pass. Adapter all-target verification passes 81 ordinary
tests with 10 explicit live-only ignores plus 4 preparation tests; Python
relay tests pass 3/3; format/diff checks pass; three reviews report P0/P1/P2=0.
No external/provider credential, real model/provider request, external
network, public listener, database, MCP, push, merge, deployment, payment,
account change, or release was used. The untracked TASK-051 script remains
untouched and excluded.

# TASK-071 Local Handoff — 2026-08-14

## Outcome

`FullChainHermes` now consumes the active, secret-free same-port recovery
receipt without resubmission. Automatic reconciliation is restricted to a
known run ID plus exact `HERMES_LOOPBACK_TIMEOUT`,
`HERMES_RUN_DEADLINE_EXCEEDED`, or `HERMES_LOOPBACK_TRANSPORT_FAILED`.
Malformed, cross-bound, cancelled, failed, HTTP-rejected, production-liveness,
and every other kind/code pair preserve the exact initial failure and never
inspect recovery state.

The first implementation (`f7cd1b3`) exposed a real RED: structural failures
also retain a receipt and could be washed into success. `fbbebbb` added the
exact eligibility and run-ID gates. A second RED showed repeated observation
uncertainty was collapsed to `HermesExecution`/gateway `Denied`; `27a7b7f`
preserves it through the existing `LATTICE_DELIVERY_RECONCILIATION_REQUIRED`
classification, while definitive reconciliation failures remain exact. No
public error, receipt, MCP, database, credential, network, or ownership schema
changed.

Runtime all-target verification passes 97 tests with one explicit
marker-owned PostgreSQL live-only ignore; the 20 composition, 1 coordination,
5 dispatch, 31 MCP, and 1 task-control integration tests all pass. Project
verification passes 48 Node tests; format and diff checks pass. Three
independent final reviews found no P0/P1 correctness or architecture findings.
No WSL, real provider/model, credential read, external network, database
mutation, push, merge, deployment, payment, account change, or release was
used. The untracked TASK-051 script remains untouched and excluded.

# TASK-070 Local Handoff — 2026-08-14

## Outcome

Hermes same-process status reconciliation now returns the existing strict
canonical reflection paired with normalized `RuntimeKind::Live` evidence from
the exact same invocation and output digest. Both the base adapter and
`ProductionHermesPort` expose the additive method; the production wrapper
retains its existing prepare, successful proxy completion, child-liveness, and
deadline gates. The prior canonical-only reconciliation API remains compatible.

The scripted ambiguity test first failed to compile with `E0599`, then proved
one submission, one status reconciliation, and exact invocation/input/output
bindings. Adapter all-target tests pass 81 tests with 9 explicit live-only
ignores, plus 4 preparation tests; Node, format, diff, code, and architecture
gates pass. No WSL, real Hermes/provider/model, credential, external network,
database, push, merge, deployment, payment, account change, or release was
used. The untracked TASK-051 script remains untouched and excluded.

# TASK-069 Local Handoff — 2026-08-14

## Outcome

`ProductionHermesPort` now exposes the adapter-owned, secret-free active
recovery receipt as a borrowed same-process observation after post-submit
ambiguity. It does not clone, persist, retry, resubmit, reconcile, launch, or
grant authority; the existing bound-port reconciliation path remains the only
consumer.

The production API seam first failed with `E0599`, then passed after the
one-line forwarding implementation. Same-process no-resubmission and timeout
fail-closed regressions pass. The adapter all-target suite passed 81 tests with
9 explicit live-only ignores, plus 4 preparation tests. Independent review
found no P0/P1 findings. No WSL, Hermes process, provider/model, credential,
external network, push, merge, deployment, payment, account change, or release
was used. The untracked TASK-051 script remains untouched and excluded.

# TASK-068 Partial Local Handoff — 2026-08-14

## Outcome

The new opt-in TASK-068 gate completed against marker-owned loopback
PostgreSQL: initial canonical reflection persistence, a physical server
stop/start, and a fresh Rust Status process replayed the exact receipt digest.
The initial phase observed one ready, research, and persist call; restart
observed zero of all three. The current contract-conflicted five-entry
autonomy baseline passes the complete Store initial/restart profile; this does
not prove the approved Registry-plus-autonomy combined profile. Runtime
all-target tests, Node verification, format, diff, and PowerShell parse checks
pass. The disposable listener and cluster root were cleaned, and the holder
receipt reached its closed terminal event.

The task remains `partial`, not integrated-complete. The inherited branch has
an autonomy receipt migration at ordinal `0005`, while accepted ADR-020 keeps
that ordinal for Project Registry. Completed Registry sources `12f7100` and
`a1aced9` are not ancestors of this worktree; autonomy source `714f3b9` occupies
the ordinal. `aa097e0`/`593112e` align only the inherited five-entry profile,
and `3ac4b1c` proves replay mechanics only. The named follow-up is **Registry
`0005` / autonomy `0006` migration reconciliation**, with durable identifier
`POSTGRES-REGISTRY-AUTONOMY-MIGRATION-RECONCILIATION`. The stop boundary is:
do not cherry-pick/merge TASK-022, reorder migrations, change schema/manifest
contracts, or mark TASK-068 completed in this slice. A separately authorized
ticket must preserve Registry `0005` and move autonomy to a later
ordinal/profile. TASK-068 made no migration or schema change and does not claim
that combined profile as proven. The untracked TASK-051 platform-acceptance
script remains untouched and excluded.

# TASK-067 Local Handoff — 2026-08-14

## Outcome

The credential-free Hermes integration gate now has a truthful two-leg
compositional acceptance at the existing production-owner boundary. The
canonical leg proves Delivery Run-only lazy activation, exact graph/provenance
validation before persistence, replay, Status zero-rerun, and fail-closed
teardown. The adapter leg actually started the pinned WSL2/bubblewrap/Hermes
v2026.8.3 runtime and completed both successful and failed turns through an
in-process fake Codex provider on loopback, without reading an external
credential or calling a real provider/model.

The live RED exposed that the provider control could receive three teardown
calls through worker, host, and Drop paths. A private `OnceLock`-backed
teardown gate now invokes the underlying control exactly once and replays the
same success or ambiguity to every caller. Both live paths assert one teardown
and removal of the owned root; startup timeout cleanup also passes.

Current evidence: adapter all-targets 81 tests plus 4 preparation tests PASS
with 9 explicit live-only ignores; runtime canonical focused tests 9/9 PASS;
the ignored pinned-runtime success, failed-turn, and startup-timeout tests were
run explicitly and PASS; format and diff checks PASS. No external credential,
real provider/model request, public network, PostgreSQL restart, push, merge,
deployment, payment, account change, or release occurred. The untracked
TASK-051 platform-acceptance script remains untouched and excluded.

# TASK-066 Local Handoff — 2026-08-14

## Outcome

Canonical production reflection orchestration now has a private deterministic
seam that preserves the existing PostgreSQL and Hermes call paths. Run proves
the exact ready, load-miss, graph-receipt, research, persist, and reload order;
fresh Status loads the exact same receipt with no Hermes activation or rerun.
A mismatched reload fails closed, and the production seal now remains true
after the ready runner becomes a bound port.

Runtime all-target tests passed serially: 96 library, 20 composition,
1 coordination, 5 dispatch, 31 MCP, and 1 task-control tests. A parallel run
hit one pre-existing timing-sensitive exact poll-count assertion; its isolated
rerun passed. Format and diff checks passed, and independent code and
architecture reviews reported no P0/P1/P2.

No PostgreSQL instance, Hermes process, provider/model, credential, network,
push, merge, deployment, or release was used. The existing untracked TASK-051
platform-acceptance script remains untouched and excluded.

# TASK-065 Local Handoff — 2026-08-13

## Outcome

Production Hermes admission no longer depends on the unexecuted
`lattice-hermes-broker` helper. Canonical configuration now requires twelve
executed-input settings, and the ephemeral production receipt uses domain v2
without helper path/hash identity. The existing direct pinned Codex launcher,
strict config, scrubbed environment, Job ownership, lazy activation, MCP tools,
PostgreSQL truth, and teardown semantics are unchanged.

The tracked TASK-037 verifier no longer treats the helper as a production
build, input, gate, or evidence hash. The legacy helper remains independently
buildable and its no-argument invocation fails closed with exit 64, empty
stdout, and fixed stderr.

Affected adapter and runtime all-target suites passed; Node verify passed
48/48; format, diff, project, and PowerShell AST checks passed. One attempted
workspace-wide run exceeded its outer bound in an unchanged fake-Codex process
test, so it is not claimed PASS; the run-owned processes were stopped and both
affected suites passed independently. No credential was read, no Hermes/model
request ran, and no push, merge, deployment, or release was performed. The
untracked TASK-051 acceptance script remains untouched and excluded.

# TASK-064 Local Handoff — 2026-08-13

## Outcome

Canonical four-tool `latticed` now owns an opt-in lazy production Hermes path.
The default and exact `TASK_ONLY` mode retain the previous behavior. Exact
`LATTICE_HERMES_MODE=PRODUCTION` creates no child at process startup; only
Delivery Run activates the governed production runner, before writer effects.
Delivery Status and both Task tools remain zero-Hermes paths.

Canonical MCP shutdown explicitly terminates an activated owner once, and an
ambiguous teardown cannot become exit 0. Unknown mode values are rejected with
fixed diagnostics without echoing their values. MCP discovery remains exactly
four tools with unchanged schemas. Legacy `lattice-full-chain` exit behavior is
unchanged.

Verification on the final candidate snapshot: workspace Rust tests PASS;
runtime all-targets PASS (92 library, 20 composition, 1 coordination,
5 dispatch, 31 MCP, 1 task-control); Node verify PASS 48/48; format, diff, and
project checks PASS. Independent code, test, and architecture reviews report
no P0/P1/P2. Strict runtime Clippy remains blocked by 17 pre-existing
diagnostics in unchanged `lattice-hermes-adapter` files; TASK-064 adds no
suppression.

No credential was read, no Hermes/model request was launched, and no push,
merge, deployment, or release was performed. The existing untracked TASK-051
platform-acceptance script remains excluded and untouched.

# TASK-056 Local Handoff — 2026-08-13

## Outcome

LATTICE now provides canonical `latticed --hermes-preflight`, a local,
no-effect static check for the sealed Hermes production configuration. It
reports only a fixed classification plus missing setting names; it never
exposes values, paths, credentials, or raw errors, and it never launches an
external process. A statically accepted configuration is explicitly
`CONFIGURATION_PRESENT_UNVERIFIED`, not ready or launch-authorized.

On this machine the preflight reports missing production configuration. The
full-chain launch remains fail-closed until the approved pinned runtime,
containment, broker, and secret source pass their normal launch-time checks.
The legacy `lattice-full-chain` observer still rejects extra arguments. This
does not change the no-argument commands, MCP surface, or already verified
TASK-050, TASK-054, and TASK-055 behavior.

Status: `DONE` for the local TASK-056 product preflight on branch
`feature/task-051-p0-platform-live-acceptance`; push, PR, merge, deployment,
and release were not performed.

- Independent review P1-P2 are addressed: the preflight moved off the legacy
  observer and parsed configuration no longer reports ready. The P3 follow-up
  retains unavailable-manifest redaction and adds a separate test-owned exact
  pinned-manifest fixture whose digest is checked before `latticed` reaches an
  invalid-secret sentinel. Both tests assert exact fixed stderr, no value leak,
  and the new fixture is scope-cleaned. Independent re-review found no P0, P1,
  P2, or P3 findings.

- `lattice-orchestrator` now owns the pure typed work/evidence projection,
  fail-closed dispatch round, next-round recomputation, and archive/retain
  recommendation. `lattice-runtime::coordination` is the normal product gate.
- Structural ambiguity rejects the round; unknown/blocked/incomplete work is
  not dispatched; both sides of a current resource conflict are blocked.
- Dispatch output is data-only and bound to the existing governed LATTICE
  execution path. No MCP, window/process control, I/O, persistence, credential,
  deployment, or self-modification surface was added.
- Current verification: workspace Rust tests PASS; Node 48/48 PASS; runtime
  81/81 PASS; orchestrator tests and strict Clippy PASS; format/project check
  PASS. Full strict runtime/workspace Clippy remains blocked by pre-existing
  unchanged runtime/Hermes diagnostics.
- Review: no TASK-055 findings; independent reviewer separation is not proven.
  The pre-existing untracked TASK-051 script was preserved and excluded.

# Previous GitHub Handoff — 2026-08-10

Status: `LOCAL_ACCEPTED / CHATGPT_TUNNEL_PENDING` for SPEC-003 v4 / TASK-038
Phase 2. Production `latticed`, real PostgreSQL Writer Lease/fencing, governed
Codex, verification/Git, physical database restart, and fresh-process durable
Status passed the canonical-local gate. A refreshed real ChatGPT Secure MCP
Tunnel discovery/invoke is still required. TASK-037 production repair remains
intentionally second.

Repository: `z72124223/lattice-devos`<br>
Branch: `feature/task-038-chatgpt-mcp`<br>
Upstream: none; local stable checkpoint not yet pushed<br>
Base: local Phase 1 checkpoint `512732d`<br>
Worktree:
`C:\Users\f7212\Documents\Codex\2026-07-29\lattice-worktrees\chatgpt-mcp`

## User-Corrected Execution Order

1. Complete bounded GPT -> LATTICE -> Codex dispatch under TASK-038.
2. Then resume TASK-037 production-chain diagnosis and repair.
3. Claim final combined production acceptance only after both gates pass.

TASK-038 Phase 2 must add `lattice_task_submit` and `lattice_task_status` to
the same `latticed` One Gateway. The first intent is exactly
`CONTROLLED_CODEX_CANARY`; LATTICE, not GPT, owns the complete Task Spec,
lease/fence, workspace, Codex prompt/profile, verification, Git, audit, and
status binding. No Project Registry or Policy authority is fabricated for this
fixed canary; broader templates remain closed until that live composition
exists.

## Retained Phase 1 Evidence

- The private Secure MCP Tunnel is able to launch `latticed`; ChatGPT refreshed
  and discovered exactly `lattice_delivery_run` and
  `lattice_delivery_status`.
- Both existing tools have closed zero-argument schemas. Legacy MCP
  `2025-11-25` and stateless `2026-07-28` reach the same composition-owned
  binding.
- The earlier focused runtime/tunnel/governance checks passed at the Phase 1
  checkpoint. Those results prove transport/discovery only and are not Phase 2
  or production-execution evidence.

## Approved Phase 2 Boundary

- ADR-023, SPEC-003 v4, and TASK-038 select closed process-start ingress
  evidence. Production Secure MCP Tunnel and local canonical acceptance use
  distinct non-substitutable commitments. MCP `clientInfo`, model text, and
  caller identity fields cannot grant authority.
- Submit accepts only `CONTROLLED_CODEX_CANARY` plus one bounded
  `client_request_id`. Status accepts only the returned lowercase SHA-256
  `task_ref`. The public schema
  contains no arbitrary task/prompt, shell, SQL, path, Git/test command,
  credential, provider, actor/session, lease/fence, or Codex-thread control.
- The server builds and Task Domain validates the complete Task Spec 2.1. Its
  one digest must bind Gateway, Task Ledger, Writer Lease, Codex,
  verification/Git and status evidence.
- PostgreSQL is authoritative for task lifecycle, exact idempotency,
  fixed-profile audit, Writer Lease/fencing, outcomes,
  and status.
- Writer Lease 1.1 owns snapshot/checkpoint bytes and the repository trait. New
  `postgres-writer-lease` 1.0 owns physical persistence through independent
  `db/extensions/writer-lease/v1.sql`.
- Postgres Store 1.6 verifies the exact combined
  `V3CodebaseMemoryV2WriterLeaseV1` catalog/ACL profile and may invoke only the
  fixed same-transaction 15-field current-authority predicate for a fenced
  Task Ledger append. It does not install, mutate, parse, depend on lease
  crates, or own Writer Lease state.
- Global migrations `0001` through `0004` and Codebase Memory v2 extension are
  unchanged; no `0005` is introduced.
- Task Submit/Status is `WriterOnly` and must leave Graphify/Hermes/Memory
  footprints at zero. The `lattice_delivery_run` MCP tool reaches the same
  governed writer and starts downstream only after durable Task completion and
  lease release; the non-MCP compatibility command remains scripted-only.
  Alternate `lattice-full-chain` is a read-only delivery observer: Run is
  fixed-denied before dispatch and task tools are absent/unknown under both MCP
  generations.
- The fixed canary is limited to a 300-second Task deadline, reserves 30 seconds
  for finalization, and uses a 600-second lease TTL. Longer profiles remain
  closed until heartbeat, governed interruption, and orphan recovery exist.

## Current Governance Checkpoint

- New ADR: `docs/adr/ADR-023-bounded-mcp-task-dispatch-and-postgres-writer-lease.md`.
- SPEC-003 is version 4; TASK-038 remains `in-progress` only because the real
  ChatGPT tunnel/session gate is still open.
- Versioned constitutions: `latticed` 1.4, Contracts 1.12, Ports 1.8,
  Task Domain 2.2, Orchestrator 2.3, Writer Lease 1.1, PostgreSQL Writer Lease
  1.0, Postgres Store 1.6, and Codex Adapter 1.2.
- Canonical-local acceptance ID: `8c21e96b9bc44b1d87de0dea884b9678`.
- Evidence root:
  `target/lattice-delivery/8c21e96b9bc44b1d87de0dea884b9678/evidence`.
- Canonical `latticed` SHA-256:
  `130ef9f92f2582055d9828828c95526a58f01aa7772e43c4db31062219d278b2`.
- Official Codex 0.146.0 SHA-256:
  `bc343ba420dc2e2e9f59e6fc5e5bf0aae1cd8c771fc319665241fc9c0271fddb`.

## Canonical-Local Acceptance Evidence

- Production `latticed` advertised exactly four legacy/stateless tools with
  closed task input and six-field output schemas. Prohibited authority,
  shell/SQL/path/Git/credential/lease/thread properties failed before dispatch.
- Submit and a fresh second `latticed` Status returned the exact same public
  result: task ref
  `ab8724dd51419cf190ad491f1f8973894bca56dc0c3aed55ebc3723f6214177d`,
  ledger head
  `f3d3b84625f80f26e90b6ed06514bb4f7e8a65f112c089fb6d8a3c82b7d6cdc2`,
  result `457bab1f71b5bd69e99f3240ca170a25fb88895b94474efd7d747916b2c86bcd`.
- PostgreSQL 17.10 physically restarted with the same system identifier and a
  new postmaster start time before the fresh Status request. Database, Git, and
  timestamp-sensitive Codex Home after-submit/after-status evidence files are
  pairwise byte-identical.
- Task Ledger contains one TaskCreated, eight legal transitions, one Codex
  effect intent, one verified outcome, and one result. Writer Lease has fence
  high-water `1`, two immutable commands, two transitions, and no current
  writer. Exact retry was identical and different-key substitution was denied.
- Codex ran once, fixed verification ran once, and Git produced one governed
  task commit. Status caused zero rerun. Graphify/Hermes/Memory effect delta is
  zero for the WriterOnly capability.
- Both process jobs reported `ActiveProcesses=0`; the execution home was
  removed and its read-only credential source remained unchanged.
- Live Store profile and Writer Lease restart/concurrency/fault suites passed
  without `SKIP:`. Workspace tests, changed-slice strict Clippy, format,
  `npm.cmd run verify`, PowerShell static/tunnel tests, and diff-check pass.
- Fresh Status and pre-existing `Merging + result` recovery independently
  replay Writer Lease snapshot/checkpoint and physical command/transition rows.
  Completed requires released `1/2/2`; recovery permits only active `1/1/1` or
  released `1/2/2`, before any additional Task Ledger mutation.
- Full workspace strict Clippy remains a recorded baseline exception: eleven
  lints are in unchanged `lattice-hermes-adapter`, outside TASK-038's bounded
  TASK-041/042-owned slice.

## Required Evidence Still Open

- Refresh the actual Secure MCP Tunnel against this stable feature checkpoint.
- In a real ChatGPT session, discover and invoke `lattice_task_submit`.
- In a separate new ChatGPT request/session, invoke `lattice_task_status` and
  read the same PostgreSQL durable terminal result.
- Do not relabel the passing scope
  `LOCAL_CANONICAL_MCP_NOT_CHATGPT_TUNNEL` as ChatGPT evidence.

## Deferred TASK-037 Truth

- The newest ChatGPT observation is
  `LATTICED_DATABASE_CONNECT_REJECTED`; this is the current first visible
  production-chain failure and is not assumed to share the older Hermes cause.
- Older formal evidence remains
  `HERMES_PRODUCTION_CHILD_EXITED / LATTICE_HERMES_REFLECTION_REJECTED` after
  reaching the later Hermes/Codex-broker boundary.
- TASK-037 was not repaired or re-accepted in this Phase 2 governance update.
  After TASK-038 completes, diagnosis must start from the then-current first
  failure and proceed PostgreSQL -> Codex -> Graphify -> Hermes -> Memory ->
  Status. No production E2E PASS is claimed.

## Exact Next Action

1. Commit the current clean stable checkpoint and publish the provider-neutral Task,
   Writer Lease, process-start, and Codex thread/turn evidence boundary to
   Issue #6; Issue #2 may consume it only as passive observation.
2. Refresh the real Secure MCP Tunnel and complete the two-session ChatGPT
   submit/status gate without changing or weakening the canonical-local proof.
3. Only after TASK-038 Phase 2 acceptance, resume TASK-037 from the actual first
   production blocker.

No CI, PR, merge, tag, release, deployment, public exposure, or
credential/account change is claimed. The exact feature branch may be pushed
only after the authorized publish preflight; any missing GitHub authentication
must be reported as `PUSH_PENDING_AUTH`. Historical TASK-037 sections below
are retained for incident evidence; this top section is authoritative.

# TASK-037 Production Gate Handoff — 2026-08-09

Status: `NEEDS_REVIEW` for TASK-037. The formal
`Hermes -> Memory -> Status` acceptance has not passed.

Repository: `z72124223/lattice-devos`<br>
GitHub URL: `https://github.com/z72124223/lattice-devos`<br>
Branch / upstream: `feature/task-037-full-chain-integration` /
`origin/feature/task-037-full-chain-integration`<br>
Implementation checkpoint: `90f44cc17b5c60a339a59926709e8672f576ec29`<br>
Remote-only checkpoint preserved: `dba4ebe` (`docs/roadmap/linear-integration.md`)<br>
Synchronization merge: `8898a79b10bdc60bee47c45d62a0794dd4cfe408`<br>
Workspace: expected clean after the handoff commit.

## TASK-037 Current Truth

- Nineteen local commits after `1b97f32` added the formal verifier and bounded
  Hermes/Codex broker repairs. The changes preserve the two-tool MCP surface,
  PostgreSQL truth, and the exclusive product-code writer boundary.
- The latest clean acceptance copy is
  `target/full-chain-acceptance/969785dbfa0e4db4a4d4f69cb3153840`.
  Its `params.json` binds the correct branch, repository root, clean starting
  tree, and exact implementation HEAD `90f44cc`.
- The pre-Run status correctly failed closed with
  `LATTICE_DELIVERY_RECONCILIATION_REQUIRED`, and the post-reset Memory probe
  contained zero analyses, receipts, audits, records, reflections, and OpenClaw
  commands. This is evidence against the earlier verification-copy binding
  hypothesis; it is not full-chain success.
- The Run spawned and bound the Codex broker child and verified its post-spawn
  identity, then the child exited. Redacted evidence is
  `HERMES_PRODUCTION_CHILD_EXITED`, stderr byte count `354`, and stderr SHA-256
  `9dc173682b03ca48eaa6e2f1deb5706d4a7a265e4f9bfb4cc4a60ac80ed9797f`.
  The outer result is `LATTICE_HERMES_REFLECTION_REJECTED` with
  `tool_is_error=true`.
- Because Run failed, no post-Run Memory write/read or Status success query was
  performed. No PASS marker exists. Processes and listeners stopped, and the
  OpenClaw profile and temporary delivery Codex home were removed; failure
  isolation roots were retained for diagnosis.

## Verification At Checkpoint

- `cargo fmt --all -- --check`: exit 0.
- `cargo test -p lattice-hermes-adapter --locked`: exit 0; 65 passed, 7 ignored.
- `cargo test -p lattice-runtime --test composition --locked`: exit 0; 8 passed.
- `git diff --check`: exit 0.
- `powershell -NoLogo -NoProfile -NonInteractive -ExecutionPolicy Bypass -File
  scripts/run-task037-full-chain-verification.ps1`: failed closed at
  `TASK037_FULLCHAINRUN_TOOL_ERROR`; acceptance ID
  `969785dbfa0e4db4a4d4f69cb3153840`.
- Remote CI, primary-branch merge, deployment, and release were not performed.

## Material TASK-037 Files

- `scripts/run-task037-full-chain-verification.ps1`
- `scripts/run-task019-postgres.ps1`
- `apps/lattice-runtime/src/composition.rs`
- `crates/lattice-hermes-adapter/src/{broker,containment,lib,production}.rs`
- `crates/lattice-hermes-adapter/src/{hermes_sandbox_runner,wsl_outer_runner}.py`
- `crates/lattice-hermes-adapter/tests/reflection_api.rs`
- `crates/lattice-graphify-adapter/src/identity.rs`
- `crates/lattice-graphify-adapter/tests/exact_git_snapshot.rs`

## Exact Next Action

1. Start from this feature branch and inspect the redacted child-exit boundary
   in acceptance `969785dbfa0e4db4a4d4f69cb3153840`.
2. Determine why the sealed `codex.cmd` child still exits after the Node PATH
   repair. Add only fixed, non-secret classification if more evidence is
   needed; do not persist raw stderr, credentials, prompts, or URLs.
3. Apply the minimum fix, run focused tests, then run the formal verifier once.
   Query post-Run Memory and Status only if Run succeeds.
4. TASK-038 may proceed through MCP compatibility, bounded transport/adapter,
   identity, and contract-test work before TASK-037 passes. Its Phase 1
   decision is recorded in `docs/roadmap/TASK-038-MCP-COMPATIBILITY.md`.
   TASK-037 remains the production end-to-end acceptance gate only.

Historical TASK-037 sections below are preserved for incident diagnosis; the
TASK-038 section at the top is the authoritative cross-session handoff.

# LATTICE DevOS TASK-033 Graphify/PostgreSQL Memory Checkpoint Handoff

## Status And Alignment

`DONE` for the bounded TASK-033 Graphify -> same-database PostgreSQL Codebase
Memory -> database restart -> exact query/receipt replay -> `latticed`
composition checkpoint. Hermes and OpenClaw were not started. TASK-032 official
Codex live remains `FAILED_DIAGNOSTIC`; no official live retry, sandbox setup,
unelevated/no-sandbox mode, deployment, push, or merge was attempted.

## Combined Production Checkpoint — 2026-08-05

- `lattice_delivery_run` now executes the pinned Graphify adapter against the
  exact committed scripted fixture, binds the durable delivery receipt to
  project/TASK/commit, and persists only through `PostgresCodebaseMemory` fixed
  functions. `lattice_delivery_status` reconstructs the same typed request and
  calls PostgreSQL receipt replay through a fresh runtime-role connection; it
  does not invoke Graphify.
- `powershell.exe -NoProfile -ExecutionPolicy Bypass -File
  .\scripts\run-lattice-delivery.ps1` exited 0 in 254 seconds. It emitted
  `TASK019_POSTGRES_HARNESS=PASS` and `LATTICE_DELIVERY_HARNESS=PASS` on
  PostgreSQL 17.10 at an ephemeral non-5432 endpoint.
- The combined run proved Store+Memory initial/restart, exact V3+Memory
  catalog/ACL admission, real non-empty Graphify persistence/retrieval, a
  second PostgreSQL stop/start, exact run/status graph-field equality, and an
  unchanged snapshot/staging footprint during fresh-process status. The final
  graph receipt digest was
  `b118e01deeab76c65c2a19a2dfb44c2f67723c7b39652fa2414cdaaeaa021a88`.
- Focused production verification passed: runtime all-target tests (30 unit,
  7 composition, 5 dispatch, 11 MCP), strict runtime Clippy, format, and the
  combined live gate. Broader non-P0/P1 review and redundant full matrices are
  intentionally deferred to the integration window.

## PostgreSQL Memory Checkpoint — 2026-08-05

- Added the independent `postgres-codebase-memory` crate and exact embedded
  `db/extensions/codebase-memory/v1.sql` profile: six owned tables, identity
  ledger, explicit admin runner, and three fixed `SECURITY DEFINER` functions.
- Added typed database/extension identity to durable contracts, validated
  restart replay constructors, and the production `PostgresCodebaseMemory`
  adapter. It accepts no SQL, path, schema, DSN, credential, or MCP input.
- RED evidence: the adapter and three replay constructors were absent and the
  focused tests failed to compile. GREEN evidence: adapter API 1/1, normalized
  contract replay 5/5, package all-target tests, and strict focused Clippy pass.
- Live evidence:
  `powershell.exe -NoProfile -ExecutionPolicy Bypass -File
  .\scripts\run-task019-postgres.ps1 -MemoryOnly` exited 0 in 19.1 s on
  PostgreSQL 17.10 at an ephemeral non-5432 endpoint. It proved partial and
  collision rejection, transactional rollback, exact install/no-op,
  catalog/ACL verification, runtime direct-table denial, real analysis and
  retrieval persistence, exact retries, changed-request rejection, stop/start,
  and fresh-process exact receipt replay.
- The full Store restart matrix and `lattice_delivery_status` composition now
  pass in the combined checkpoint above.

## Delivered

- Added typed graph-memory requests/evidence, ports, pure Codebase Memory
  normalization/retrieval, and pure orchestrator ordering with fail-closed
  zero-later-effect behavior.
- Added an exact tracked-Git snapshot materializer that excludes untracked and
  secret-shaped paths, binds the commit/tree/manifest, and rejects drift.
- Pinned `graphifyy` 0.9.33, upstream commit
  `4e7e6b1f7e0df10ed07d5f28f9189bbde42940f1`, Apache-2.0, wheel SHA-256
  `c32b5792c783a6e66b1100b35bc65df3538e3f69b9df45fb098c9634c1b8eb01`,
  and the complete reviewed dependency payload.
- Added the Windows/WSL Graphify adapter: fixed headless arguments, cleared
  provider environment, private tmpfs runtime/source/output, exact copy
  verification, strict seven-field framed copy-out, exclusive parent capture
  handles, bounded teardown, and Landlock ABI 3 truncate enforcement before
  any Graphify command.
- Preserved Store/ADR-020 Project Registry ownership and the global migration
  bytes/manifest. The independent Memory extension and runtime composition add
  no third MCP tool; Hermes and OpenClaw remain absent from this checkpoint.

## Material Files

| Files | Reason |
|---|---|
| `crates/lattice-contracts`, `crates/lattice-ports` | typed graph, memory, analysis, persistence, and retrieval boundaries |
| `crates/lattice-codebase-memory` | deterministic untrusted structural record normalization and retrieval |
| `crates/lattice-orchestrator` | pure snapshot -> analyze -> validate -> persist -> retrieve effect order |
| `crates/lattice-graphify-adapter` | exact Git snapshot plus pinned, contained, fail-closed Graphify execution |
| `crates/lattice-postgres-codebase-memory`, `db/extensions/codebase-memory/v1.sql` | sole same-database Memory persistence owner and exact extension bytes |
| `apps/lattice-runtime`, `scripts/run-lattice-delivery.ps1`, `scripts/run-task019-postgres.ps1` | production run/status composition and restart acceptance |
| `Cargo.toml`, `Cargo.lock` | register the new Rust crates and locked dependencies |
| SPEC-002, ADR-022, affected constitutions, TASK-033, `PLANS.md` | approved boundary and completed database/composition checkpoint |
| `HANDOFF.md`, `docs/workflow/WORKFLOW_LEDGER.md` | durable checkpoint truth and continuation evidence |

## Verification And Review

- Private typed Graphify live: exit 0, 1 passed/0 failed, test time 112.92 s
  (115.8 s wall); exact Git fixture produced deterministic typed evidence. This
  run proved the private-copy/framed-capture revision before the final ABI-3
  gate was added; the full Graphify live was not rerun after that review repair.
- Final containment repair: embedded runner SHA-256
  `98d0411709927a5687315f64efc6673a77f2241e2db6df8bd17c34886e3c2ad9`;
  execution identity
  `f270004749c7f4fc260dfc09925b52f3b7071bcc64ba5f7cbd9bd37ae1400dd5`.
  A bounded bubblewrap tmpfs probe exited 0 with
  `abi=7 truncate_denied=1 allowed_write=1`; no target Windows or WSL process
  remained.
- `cargo test -p lattice-graphify-adapter --lib --tests --locked`: exit 0;
  18 unit tests passed/2 ignored, 2 Git tests passed/2 ignored, and one static
  containment test passed/3 ignored.
- `cargo fmt --all -- --check`, strict adapter Clippy, locked full workspace
  tests, strict full workspace Clippy, and `npm.cmd run verify`: exit 0; Node
  tests 44/44. The first full run observed one pre-existing scripted Codex EOF
  timing mismatch; its exact test and the complete workspace rerun both passed.
- Three independent final read-only reviews: code P0=0/P1=0, security
  P0=0/P1=0, architecture P0=0/P1=0. Graphify Adapter 1.1 now matches the
  user-directed private tmpfs/Landlock/framed-capture implementation.

## Remaining Boundary

- The two declined-cleanup diagnostic directories still exist and are
  non-blocking; cleanup was not retried:
  `C:\Users\f7212\AppData\Local\Temp\lattice-graphify-live-typed-ports-7788-1`
  and
  `C:\Users\f7212\AppData\Local\Temp\lattice-graphify-live-typed-ports-16628-1`.
- The next executable node is a separately bounded Hermes candidate/reflection
  integration; it receives no authority from this checkpoint and was not
  started here.
- Branch: `feature/v2-rust-postgres-bootstrap`; checkpoint parent:
  `79096b6b5f184a47d44bbbd20a575bad79a5e393`; checkpoint is this commit. The
  repository has no remote, so remote synchronization, CI, branch protection,
  push, and merge remain unavailable/not performed.

---

# LATTICE DevOS TASK-032 Executable Delivery Handoff

## Status

`NEEDS_REVIEW` for TASK-032 completion. The trusted scripted delivery node is
implemented and fully verified, but official Codex live remains
`FAILED_DIAGNOSTIC`; TASK-032 and MVP-1 must remain open.

## Objective And Alignment

This checkpoint implements the approved bounded chain:

> fixed delivery intent -> Rust `latticed` -> PostgreSQL -> Codex app-server
> protocol -> isolated workspace -> one fixed test -> local Git commit ->
> PostgreSQL outcome/receipt -> restart status replay

It remains a general autonomous AI development platform. No playmate website,
deployment, publication, payment, public exposure, or protected release work
was added. OpenClaw, Graphify, Hermes, and Codebase Memory were not started
because the official-Codex prerequisite is still blocked.

## Completed And Confirmed

- Recorded the user-approved Contracts 1.10, Ports 1.6, pure Orchestrator 2.1,
  `latticed` 1.0, two-tool MCP, compatibility, and allowlist amendment in the
  existing spec, ADR, module constitutions, ticket, and plan.
- Added typed delivery request/evidence/receipt contracts and ports. The pure
  orchestrator depends only on `lattice-contracts` and `lattice-ports` and
  orders intent -> workspace -> Codex -> scope -> fixed test -> Git -> outcome
  -> receipt, stopping on every known or ambiguous failure.
- Added the canonical `latticed` composition root, concrete PostgreSQL,
  workspace/test/Git, and Codex adapters, plus the compatibility command that
  delegates to the same composition.
- Exposed exactly `lattice_delivery_run` and `lattice_delivery_status` over
  bounded MCP stdio. Both tools have closed zero-argument schemas; arbitrary
  shell, SQL, path, credential, provider, and task inputs are absent.
- Added typed PostgreSQL v2 restart reconstruction, complete non-secret
  configuration binding, post-mutation ambiguity handling, shared delivery
  deadlines, one fixed verification before commit, and safe empty `.agents`
  handling.
- Closed the final review blocker by checking the absolute deadline after Git
  child completion/output reads and before commit evidence returns. After a
  durable intent, every outcome-persistence failure now remains
  reconciliation-required, including ambiguous commit plus known DB timeout.
- Bound `SCRIPTED_ACCEPTANCE` to the checked-in fixture bytes, canonical paths,
  launcher/server hashes, exact wrapper, and marker before any database or
  process effect. A self-consistent tampered server cannot claim that mode.
- Added PowerShell and Rust fail-closed incident gates that reject official mode
  before build, database, or child-process effects.

## Material Files Changed

| Files | Reason |
|---|---|
| `Cargo.lock`, `apps/lattice-runtime/Cargo.toml`, `crates/lattice-codex-adapter/Cargo.toml` | lock and declare the concrete composition dependencies |
| `crates/lattice-ports/src/lib.rs` | add typed delivery effect ports |
| `crates/lattice-orchestrator/src/lib.rs`, `crates/lattice-orchestrator/tests/delivery_order.rs` | pure effect ordering and failure/ambiguity regressions |
| `crates/lattice-codex-adapter/src/{lib,delivery,process}.rs`, adapter tests | bounded identity/protocol/process adapter with one absolute deadline |
| `apps/lattice-runtime/src/{lib,composition,delivery_ledger,git_delivery,mcp}.rs` | canonical composition, durable replay, fixed test/Git adapter, and two-tool MCP |
| `apps/lattice-runtime/src/bin/latticed.rs`, runtime tests | canonical executable and composition/CLI/MCP regressions |
| `apps/lattice-runtime/src/fixtures/task032-scripted-codex.ps1` | immutable trusted scripted app-server fixture |
| `scripts/run-lattice-delivery.ps1`, `package.json` | bounded acceptance harness, official incident gate, and project-only Node test discovery |
| `PLANS.md`, SPEC-002, ADR-021, affected module constitutions, TASK-032 | minimal approved architecture and current incident/acceptance state |
| `docs/workflow/WORKFLOW_LEDGER.md`, `HANDOFF.md` | durable evidence and continuation boundary |

## Verification Evidence

- `cargo fmt --all -- --check`: exit 0.
- `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`:
  exit 0, zero warnings.
- `cargo test --workspace --all-targets --all-features --locked`: exit 0; all
  workspace test binaries pass, including 7 composition and 11 MCP tests.
- `npm.cmd run verify`: exit 0; project check passes and Node tests are 44/44.
- `git diff --check`: exit 0; one non-failing LF-to-CRLF notice only.
- PowerShell AST parse for the harness and checked-in fixture: pass.
- `cargo tree -p lattice-orchestrator --depth 1`: only Contracts and Ports.
- Trusted scripted fixture
  `target/lattice-delivery/c9bf2939ad5844e9973ee0af0a84b756`:
  PostgreSQL 17.10 initial/restart pass, status `COMPLETED`, runtime
  `SCRIPTED_ACCEPTANCE`, clean fixture repository, and commit
  `ed408cc4373519f57950a66660148df39f9d5f82` changing only `answer.txt`.
- The run/status evidence agrees on request, profile, configuration, intent,
  outcome, and receipt digests; final evidence is
  `target/lattice-delivery/c9bf2939ad5844e9973ee0af0a84b756/evidence/final.json`.

## Official Live Failed Diagnostic

- The official attempt is not acceptance evidence. Windows displayed
  `codex-windows-sandbox-setup.exe` with "The specified module could not be
  found" while the isolated fixture contained an uncommitted `answer.txt`.
- The OpenAI-signed x64 helper SHA-256 is
  `7191d24f6fb4a26cbbce0d2aecd6deb71fa074a8cb5f24a45d2fa2164473885f`;
  its direct system imports were resolvable. OpenAI issues
  [#29952](https://github.com/openai/codex/issues/29952) and
  [#29200](https://github.com/openai/codex/issues/29200) report the same open
  Windows sandbox-helper regression/compatibility failure.
- Read-only package evidence identifies `@openai/codex` 0.144.6 and signed
  native `bin/codex.exe` SHA-256
  `4b76ded066d0239115ca97473d010c92072bc5c5550a45dd7cbebe1e9eb956a7`.
  The helper's exact path is
  `C:\Users\f7212\AppData\Roaming\npm\node_modules\@openai\codex\node_modules\@openai\codex-win32-x64\vendor\x86_64-pc-windows-msvc\codex-resources\codex-windows-sandbox-setup.exe`.
- No exact missing DLL, helper stderr, or helper exit code was captured; none is
  claimed. The preserved diagnostic fixture is
  `target/lattice-delivery/1b1e1661d9e843e2b9e4774b93bf0dc9` at initial
  commit `94ba7385b81dd607c8a271a3c988e0f9bc82fac1`, with untracked
  `answer.txt` and no delivery commit.
- No official live or sandbox setup was retried after the user stop, no
  unelevated/no-sandbox mode was selected, and no system component was changed.

## Workflow And Review Ledger

| Stage | Status | Evidence |
|---|---|---|
| Repository/Git inspection | valid | feature branch, base `4cf98cf`, dirty candidate preserved, no remote |
| Clarification/spec/ticket | valid | prior explicit approval reused; SPEC-002 v26 and TASK-032 remain current |
| Module governance | valid | approved versioned constitutions and ADR-021; no silent boundary change |
| TDD implementation | valid | observable red/green coverage for ordering, deadlines, replay, protocol, Git, and fixture trust |
| Focused/full verification | pass | Rust format/Clippy/tests, Node 44/44, AST, diff and dependency checks |
| Scripted runtime acceptance | pass | real PostgreSQL restart plus real isolated Git commit; explicitly not official live |
| Independent code review | pass | final P0=0/P1=0/P2=4/P3=1; non-blocking findings recorded below |
| Architecture review | pass with debt | pure ports boundary and two-tool surface preserved; P0=0/P1=0 |
| Integration/CI/merge | partial/blocked | local combined checks pass; no remote, CI/branch protection, push, or merge authorization |
| Handoff | complete for checkpoint | this file plus `docs/workflow/WORKFLOW_LEDGER.md` |

## Remaining Gaps And Next Step

- P2: PostgreSQL sets remaining session timeouts at connection, but lower-level
  transactions can replace them with fixed 5s/30s values. A return after the
  absolute deadline is safely `Ambiguous`, yet latency can exceed the caller
  deadline.
- P2: Windows Codex cleanup uses unbounded `taskkill.exe.status()`/`child.wait()`
  after protocol completion; it cannot falsely report success but could delay
  terminal persistence.
- P2: Codex stdout uses an unbounded channel and `read_line` allocates before
  the 8 MiB line check. The trusted fixture cannot exploit this, but bounded
  framing/backpressure is required before removing the official incident gate.
- P2: the child-environment denylist misses exact generic names such as
  `API_KEY`, `TOKEN`, `SECRET`, and `PASSWORD`. No leak was observed and the
  pinned fixture does not inspect them, but official mode should use a narrow
  environment allowlist before its gate is removed.
- P3: MCP `initialize` accepts an empty `clientInfo` object instead of requiring
  non-empty `name` and `version`.
- The global project-memory router path was missing (`MODULE_NOT_FOUND`), so
  that documented global routing gate is unavailable in this environment.
- Wait for an upstream helper correction or an explicit user decision on a new
  safety posture before any official live retry. Only after official
  modify/test/commit/restart evidence passes may TASK-032 close and the bounded
  OpenClaw, Graphify, Hermes, and Codebase Memory nodes begin.
- No push, merge, publication, deployment, payment, account/credential change,
  or production mutation was performed.

## Restart Context

- Branch: `feature/v2-rust-postgres-bootstrap`; checkpoint base:
  `4cf98cf3f9e3b53d0e819139cdfd96ff457e587a`; no remote.
- Active ticket: TASK-032 (`in-progress`); active plan marker remains CURRENT.
- Begin with `git status --short --branch`, then read this section, PLANS.md,
  TASK-032, and the active incident gate. Do not retry official/sandbox setup.
- Overall goal remains active; do not mark TASK-032 or MVP-1 complete.

---

# Archived LATTICE DevOS TASK-021 Handoff

## Outcome

TASK-021 is complete. LATTICE now has its first durable domain repository:
Task Ledger 2.1 remains the pure Rust semantic owner, while Postgres Store 1.3
atomically persists each terminal command, optional event and outbox admission,
projection/checkpoint, and applied physical Store receipt in PostgreSQL.
SPEC-002 AC-03, AC-04, and AC-35 are complete.

This does not complete MVP-1, MVP-2, MVP-3, or the full platform. MVP-1 is
12/22 tickets (54.5%); AC-05 and AC-19 remain open. Outbox claim/delivery,
live resource observations, the other durable repositories, daemon activation,
OpenClaw/Codex/Graphify/Hermes/Codebase Memory live integration, Guardian
autonomy, production, release, and deployment remain later work.

LATTICE remains the user's general local autonomous AI development platform:

> OpenClaw + Rust LATTICE Core + PostgreSQL + Codex + Graphify + Hermes +
> Codebase Memory

The unrelated companion/playmate website remains excluded from the product,
architecture, roadmap, implementation, tests, and project evidence.

## Completed In TASK-021

- Task Ledger 2.1 exposes one Fake/Live pure vacant/plan/apply/checkpoint
  boundary, retains verified appended and denied commands, and derives exactly
  one outbox admission only for an appended `EFFECT_INTENT` with outcome
  `RECORDED`.
- Postgres Store 1.3 adds exact transaction-control-free schema v3 while
  keeping `0001` through `0003` byte-identical and preserving the immutable
  Store-v2 receipt profile for historical exact replay.
- Runtime receives exactly three Store-v3 and five Task-Ledger-v1 fixed
  functions, with zero direct protected-table SELECT/DML and no generic SQL,
  DSN, credential, environment-discovery, or raw-client surface.
- Each new command runs in one bounded `SERIALIZABLE` transaction. Store and
  Ledger finalization are two ordered fixed calls in that same transaction;
  the Ledger finalizer accepts only the Store terminal created by the current
  transaction and any later failure rolls both back.
- Every read/write re-observes dynamic global schema/full-manifest evidence in
  the transaction and compares it with constructor-frozen evidence. Store-v2
  receipt evidence remains separate from global-v3 evidence.
- Outbox replay verifies event digest, command ID, and request digest linkage.
  Duplicate, missing, cross-project/snapshot, checkpoint, terminal, event,
  command, or outbox corruption fails closed and is never auto-repaired.
- Explicit database responses remain known retryable or terminal outcomes;
  only no database response at commit yields `CommitOutcomeUnknown` and
  poisons the adapter. Transactions/functions use 5-second lock and 30-second
  statement timeouts.
- Fresh Store genesis is correctly distinct from the vacant Ledger checkpoint
  until the first atomic mutation. A vacant stream with a same-ID wrong-scope
  physical orphan now fails closed through the three-argument fixed read-head
  function and a direct live regression.

## Review History And Repairs

The initial independent code review blocked closure with P1=4 and P2=2:
constructor-only global evidence, overly broad unknown-commit classification,
incomplete outbox linkage checks, acceptance of a prior Store terminal,
wrong-scope physical-load gaps, and missing bounded timeouts. Architecture
review also blocked on transaction provenance and bounded failure semantics.

All findings received direct repairs/regressions. Live acceptance then exposed
and repaired the fresh Store-genesis/vacant-checkpoint finalizer mismatch and a
test-only ACTIVE-admission restart cleanup gap. Final re-review found one more
P2 vacant wrong-scope orphan read gap; the SQL/Rust/live compatibility unit was
repaired and the full PostgreSQL matrix rerun. Final code/security and
architecture reviews report P0=0, P1=0, P2=0, P3=0; local integration passes.

## Verification Evidence

- PostgreSQL 17.10 marker-owned harness: latest frozen initial and restart
  phases pass, including migration/upgrade, old Store replay, new/exact/
  changed/stale/outbox commands, same/cross-stream concurrency, rollback,
  bounded retry, commit-ack loss, coherent manifest drift, lock timeout,
  current-transaction `xmin`, ACL, wrong-scope orphan, corruption, and cleanup.
- Rust workspace: 432/432 tests across 52 binaries.
- Postgres Store package: 57/57 tests; migration contract: 15/15.
- Preserved Node verification: 44/44 tests.
- `cargo fmt` and strict workspace/all-targets Clippy: pass, zero warnings.
- `cargo audit`: 109 locked dependencies checked against 1,178 advisories;
  zero known vulnerabilities.
- `0004_task_ledger_repository.sql`: 111,742 bytes, SHA-256
  `cd658ed2f4624cd0a829c818c1cf96d8ac3829264046976bdde3b2fc7feea6e5`.
- Full four-entry manifest:
  `09c431df18ad71a4f44239a5d2ddf6b1774b8ffec06c7f9223f0e41757f3d407`.
- `0001`/`0002`/`0003` retain their TASK-020 bytes and hashes.
- No unmerged path, conflict marker, tracked whitespace error, PowerShell parse
  error, reverse adapter dependency, or temporary focus switch remains.

Review artifacts:

- `docs/reviews/WORKFLOW_AUDIT_TASK_021_2026-08-02.md`
- `docs/reviews/GOVERNANCE_REVIEW_TASK_021_2026-08-02.md`
- `docs/reviews/CODE_REVIEW_TASK_021_2026-08-02.md`
- `docs/reviews/ARCHITECTURE_REVIEW_TASK_021_2026-08-02.md`
- `docs/reviews/INTEGRATION_TASK_021_2026-08-02.md`
- `docs/workflow/WORKFLOW_LEDGER.md`

## Alignment And Enforcement Truth

TASK-021 stayed aligned with PLANS Step 6, SPEC-002 v23, ADR-019, Task Ledger
2.1, and Postgres Store 1.3. It adds durable Ledger/outbox-admission truth
without giving PostgreSQL domain meaning or adding outbox delivery, another
writer/truth/gateway, provider/product work, or the unrelated website.

| Gate | Current classification |
|---|---|
| Local Rust/Node tests, format, lint, audit, migration hashes | machine-enforced for this run |
| Disposable PostgreSQL transaction/concurrency/fault/restart behavior | machine-enforced for the exact marker-owned target |
| Fixed functions, direct-table denial, roles, ACLs, catalog and scope | machine-enforced locally plus independent review |
| Module ownership and dependency direction | independently reviewed plus local scans |
| Ticket allowlist | documented plus local scan; no clean per-ticket commit |
| Remote Rust/PostgreSQL CI and branch protection | missing/unverified |
| Primary merge readiness | blocked; no committed candidate, remote, or merge authorization |

## Git, Runtime, And Cleanup State

- Branch: `feature/v2-rust-postgres-bootstrap`; HEAD:
  `06c3954cd941dc6b743e9e2a1d4f94c3658b3ff9`.
- Primary: `main` at `d856cf7eb7be0072aeb32316e43f0b653d73dca2`;
  feature is four committed commits ahead and not behind.
- Remote/upstream: none. MVP-0 through TASK-021 remain one cumulative
  uncommitted dirty result; no merge was performed.
- No disposable PostgreSQL/Cargo/test process remains. The installed Windows
  PostgreSQL 17 service was not replaced or stopped.
- Two stopped ignored diagnostic roots remain under `target/`, both without
  `postmaster.pid` or listeners. Exact removal was blocked by local policy and
  was not bypassed; this is a disclosed hygiene note, not live database state.

## Next Bounded Slice

TASK-022 governance must freeze the durable Project Registry repository before
implementation. It must reuse Project Registry's pure identity/reconciliation
semantics and Postgres Store's schema-v3 transaction/authority boundary,
preserve one-way dependency and project isolation, and prove restart,
concurrency, drift/collision, corruption, and exact replay. It must not start
Writer Lease, Approval, Artifact, OpenClaw/Codex/Graphify/Hermes/Memory,
Guardian, production, release/deploy, or unrelated website work.

---

# Archived TASK-020 Handoff

## Outcome

TASK-020 is complete. LATTICE now has one exact live PostgreSQL 17 physical
`ControlStore`: Contracts 1.9 and Ports 1.4 preserve the fake while Postgres
Store 1.2 supplies schema-v2 migration, fixed-function runtime access,
durable apply/stale terminal receipts, exact replay, bounded pre-commit retry,
unknown-commit reconciliation, and restart evidence. SPEC-002 AC-34 is
complete.

This is a physical durability boundary, not a domain repository. AC-03,
AC-04, AC-05, and AC-19 remain open, and MVP-1, MVP-2, MVP-3, and the full
platform are not yet complete.

LATTICE remains the user's general local autonomous AI development platform:

> OpenClaw + Rust LATTICE Core + PostgreSQL + Codex + Graphify + Hermes +
> Codebase Memory

The unrelated companion/playmate website is excluded from the product,
architecture, roadmap, implementation, tests, and project evidence.

Current milestone state:

- MVP-0 Rust foundation: complete.
- MVP-1 offline control core: in progress; TASK-020 complete, TASK-021 next.
- MVP-2 exact-version local components and Codebase Memory: planned after the
  MVP-1 exit gate.
- MVP-3 Guardian-protected autonomy and upgrade: planned after the MVP-2 gate.

## Completed In TASK-020

- Preserved Store contract v1 as visibly fake/non-durable and introduced exact
  v2 live PostgreSQL persistence evidence without changing unrelated contract
  behavior.
- Changed only the synchronous `ControlStore::current_head` observation to
  explicit mutable access; Ports remains Contracts-only and driver-free.
- Kept `0001` and `0002` byte-identical and added one exact
  `0003_live_control_store.sql` expansion. Fresh targets and verified empty
  exact-v1 prefixes upgrade; drifted, partial, reordered, edited, unknown, or
  non-empty sources fail closed.
- Added exactly three fixed `SECURITY DEFINER` runtime functions with safe
  ownership/search-path/ACL properties. Runtime has no direct physical or
  terminal table access and cannot migrate or self-activate.
- Added `PostgresControlStore` over a caller-supplied authenticated client. It
  rechecks exact ACTIVE daemon authority and the locked physical head inside a
  bounded `SERIALIZABLE` transaction, and returns durable evidence only after
  successful commit.
- Exact retry remains byte-identical after admission, epoch, or head changes;
  changed transaction-ID reuse reveals no retained receipt. Unknown commit
  response returns no receipt, poisons the instance, and requires reconnect
  plus the exact request for reconciliation.
- Expanded the marker-owned PostgreSQL harness for fresh/upgrade/rollback,
  permissions, apply/stale/replay/substitution, concurrency, serialization
  exhaustion, overflow, corruption, response loss, restart, and exact cleanup.

## Verification Evidence

- Full Rust workspace: 409/409 tests.
- Preserved Node suite: 44/44 tests.
- `cargo fmt` and strict all-target/all-feature Clippy: pass with zero warnings.
- `cargo audit`: 109 locked dependencies checked against 1,178 advisories;
  zero known vulnerabilities.
- Duplicate dependency roots: zero.
- PostgreSQL 17.10 marker-owned initial/restart harness: self-test and complete
  TASK-020 live/fault/upgrade matrix pass.
- `0003_live_control_store.sql`: 29,518 bytes, SHA-256
  `00ae3eedd76704f26b1df58955d9d594c98f0ba525be93b15d8c9ebb1f2115c1`.
- PowerShell AST, source-scope, secret/DSN/raw-client/dynamic-SQL, conflict,
  temporary-marker, whitespace, dependency, and governance checks pass.
- Independent code/security and architecture reviews pass with P0-P3 all zero;
  local combined integration passes.

Review artifacts:

- `docs/reviews/WORKFLOW_AUDIT_TASK_020_2026-08-02.md`
- `docs/reviews/CODE_REVIEW_TASK_020_2026-08-02.md`
- `docs/reviews/ARCHITECTURE_REVIEW_TASK_020_2026-08-02.md`
- `docs/reviews/INTEGRATION_TASK_020_2026-08-02.md`
- `docs/workflow/WORKFLOW_LEDGER.md`

## Alignment And Enforcement Truth

TASK-020 stayed aligned with PLANS Step 6, SPEC-002 v22, ADR-018, and the
versioned Contracts/Ports/Postgres Store constitutions. It makes physical
PostgreSQL durability real without confusing it with Ledger, Registry, Lease,
Approval, Artifact, Guardian, provider/product, release, or production
authority.

| Gate | Current classification |
|---|---|
| Local Rust/Node format, tests, strict lint, audit, and migration hashes | machine-enforced for this run |
| Disposable PostgreSQL transaction/concurrency/fault/restart evidence | machine-enforced for the exact marker-owned target |
| Fixed runtime functions, direct-table denial, roles, ACLs, and catalog shape | machine-enforced locally plus independent review |
| One Gateway/Truth/Writer and dependency/domain boundaries | independently reviewed and locally scanned |
| TASK-020 ticket allowlist | documented plus local scan; no per-ticket committed diff |
| Remote Rust/PostgreSQL CI, branch protection, remote synchronization | missing/unverified |
| Primary-branch merge readiness | blocked; no committed candidate, remote, or authorization |

## Git And Scope State

- Branch: `feature/v2-rust-postgres-bootstrap`.
- HEAD: `06c3954cd941dc6b743e9e2a1d4f94c3658b3ff9`.
- Primary: `main` at `d856cf7eb7be0072aeb32316e43f0b653d73dca2`.
- Primary/V2 committed ahead/behind: `0/4`; remote/upstream: none.
- MVP-0 through TASK-020 remain one inspectable cumulative uncommitted result.
- No reset, clean, branch switch, commit, push, merge, production database or
  credential mutation, publication, deployment, deletion, or protected action
  occurred.

## Next Bounded Slice

TASK-021 must first freeze the durable Task Ledger adapter boundary before any
code change. It will add one domain-owned composition layer that persists the
canonical Task Ledger request, event/head, terminal command receipt, and
outbox admission atomically in PostgreSQL, reuses the pure Ledger verifier as
the semantic owner, and proves idempotency, concurrency, corruption denial,
unknown-commit reconciliation, and restart replay. It must not give the pure
domain crate I/O, make Postgres Store depend on a domain, grant runtime generic
SQL, or introduce OpenClaw/Codex/Graphify/Hermes/provider/product/release work.

---

# Archived TASK-019 Handoff

## Outcome

TASK-019 Postgres Store 1.1.5 is complete for its exact-manifest PostgreSQL
17 schema, permission, compatibility, and STOPPED-admission foundation.
SPEC-002 AC-33 is complete. This does not complete a live `ControlStore`,
durable domain repositories, AC-03/04/05/19, MVP-1, MVP-2, MVP-3, or the whole
platform.

LATTICE remains a general local AI development platform for the user's
computer:

> OpenClaw + Rust LATTICE Core + PostgreSQL + Codex + Graphify + Hermes +
> Codebase Memory

The unrelated companion/playmate website is not part of this product,
architecture, roadmap, implementation, or test target.

Current milestone state:

- MVP-0 Rust foundation: complete.
- MVP-1 offline control core: in progress; TASK-019 is complete.
- MVP-2 exact-version local components and Codebase Memory: planned after the
  MVP-1 exit gate.
- MVP-3 Guardian-protected autonomy and upgrade: planned after the MVP-2 gate.

## Completed In TASK-019

- Pinned the synchronous `postgres` 0.19.14 driver with default features off
  and SHA-256 0.11.0 while retaining the Contracts-only Ports boundary.
- Added a compile-time exact migration manifest. The fixed `0001` remains
  byte-identical and `SUPERSEDED`; transaction-control-free `0002` creates only
  database identity/history/compatibility, physical transaction foundations,
  and STOPPED/no-leader admission.
- Added the explicit administrative runner and read-only repeatable-read
  verifier with exact target sentinel, transaction-scoped concurrency lock,
  no-op retry, uncertain/committed-unverified reconciliation, and full catalog
  drift closure.
- Enforced real LOGIN-to-NOLOGIN capability separation, CONNECT-only bootstrap,
  cluster-wide ACL/default-ACL/ownership closure, exact protected-function
  denial, `max_prepared_transactions = 0`, and no notification authority.
- Added a marker-owned PostgreSQL 17.10 harness that uses a fresh non-5432
  loopback cluster, leaves the installed service untouched, restarts its own
  cluster, proves real LOGIN permissions, and deletes only its exact root after
  stopped-state and marker verification.
- Kept the deterministic fake and `ControlStore` unchanged; no live/durable
  receipt, domain write, self-activation, production credential, provider,
  product, or website behavior was added.

## Verification Evidence

- Postgres Store focused tests: 35/35.
- Full Rust workspace: 401/401.
- Preserved Node suite: 44/44.
- Format and strict all-target/all-feature Clippy pass with zero warnings.
- Two fresh PostgreSQL 17.10 trials report
  `TASK019_HARNESS_SELF_TEST=PASS` and `TASK019_POSTGRES_HARNESS=PASS` for both
  initial and restart phases.
- PowerShell AST, dependency tree, duplicate dependency, migration hash,
  debug-marker, temporary-artifact, diff, and governance checks pass.
- Independent code/security and architecture reviews pass with P0-P3 all zero;
  local combined integration passes.

Review artifacts:

- `docs/reviews/WORKFLOW_AUDIT_TASK_019_2026-08-01.md`
- `docs/reviews/CODE_REVIEW_TASK_019_2026-08-01.md`
- `docs/reviews/ARCHITECTURE_REVIEW_TASK_019_2026-08-01.md`
- `docs/reviews/INTEGRATION_TASK_019_2026-08-01.md`
- `docs/workflow/WORKFLOW_LEDGER.md`

## Alignment And Enforcement Truth

TASK-019 stayed aligned with PLANS, ADR-017, and Postgres Store 1.1.5. It made
PostgreSQL evidence real without confusing a schema foundation with a live
Store, domain truth, leader activation, or production release.

| Gate | Current classification |
|---|---|
| Manifest, runner, verifier, role/catalog/settings and harness behavior | machine-enforced locally |
| Disposable PostgreSQL transaction/concurrency/restart/permission evidence | machine-enforced locally twice |
| One Gateway/Truth/Writer and dependency boundaries | independently reviewed and locally scanned |
| Live physical Store and durable/domain receipts | missing/deferred to TASK-020+ |
| `cargo-audit`, remote Rust CI, branch protection | unavailable or missing/unverified |
| Merge readiness | blocked; no committed candidate, remote, or authorization |

## Git And Scope State

- Branch: `feature/v2-rust-postgres-bootstrap`.
- HEAD: `06c3954cd941dc6b743e9e2a1d4f94c3658b3ff9`.
- Primary: `main` at `d856cf7eb7be0072aeb32316e43f0b653d73dca2`.
- Primary/V2 committed ahead/behind: `0/4`; remote/upstream: none.
- MVP-0 through TASK-019 remain one inspectable uncommitted dirty result.
- No reset, clean, branch switch, commit, push, merge, production database or
  credential mutation, publication, deployment, deletion, or protected action
  occurred.

## Next Bounded Slice

TASK-020 will version the affected contracts before implementing the live
physical PostgreSQL `ControlStore`. It must revalidate exact ACTIVE daemon
authority and physical head in the same transaction, retain exact terminal
receipts for reconciliation, expose only narrow runtime operations, and keep
all Registry/Ledger/Lease/Approval/Artifact legality and Guardian activation
outside this slice.

---

# Archived TASK-018 Handoff

## Outcome

TASK-018 Postgres Store 1.0 is complete for its typed zero-I/O MVP-1 boundary.
SPEC-002 AC-32 is complete. This does not complete durable PostgreSQL
AC-03/04/05/19, MVP-1, MVP-2, MVP-3, or the whole platform.

LATTICE remains a general local AI development platform for the user's
computer:

> OpenClaw + Rust LATTICE Core + PostgreSQL + Codex + Graphify + Hermes +
> Codebase Memory

The unrelated playmate website is not part of this product, architecture, or
roadmap. Ten old `ACCESS_PLAYMATE` strings remain only as explicit V1
compatibility/denial fixtures; active V2 and TASK-018 paths have zero coupling.

Current milestone state:

- MVP-0 Rust foundation: complete.
- MVP-1 offline control core: in progress; TASK-018 is complete.
- MVP-2 exact-version local components and Codebase Memory: planned after the
  MVP-1 exit gate.
- MVP-3 Guardian-protected autonomy and upgrade: planned after the MVP-2 gate.

## Completed In TASK-018

- Contracts 1.8 adds bounded canonical Store transaction, daemon, scope,
  authority, physical-head, commitment, disposition, and receipt values.
- Ports 1.3 exposes only typed `ControlStore::transact` and `current_head`
  through Store-specific failures and remains Contracts-only.
- Postgres Store 1.0 implements a deterministic in-memory fake with Store-owned
  genesis/head hashes, complete request hashing, exact replay, changed-ID
  substitution denial, project/snapshot/owner/aggregate isolation, atomic head
  and receipt apply, stable stale denial, bounded capacity/serialization, and
  explicit before/after-apply fault reconciliation.
- Every receipt is fixed to `RuntimeKind::Fake` and `NonDurableFake`; no driver,
  SQL, connection, migration runner, or durable constructor exists.
- Project governance now rejects duplicate ticket IDs, invalid current-marker
  cardinality, a marker without its unique ticket, a current ticket without its
  module constitution, non-canonical constitution paths, and duplicate module
  IDs without forcing future modules active early.

## Review Repairs

Independent review found and closed bounded/canonical snapshot identity,
replay-integrity ordering, changed-ID substituted-scope probing, arbitrary
physical-head injection, revision-zero genesis override, canonical constitution
path, and stale-disposition documentation drift. Each behavioral finding
received a RED/GREEN regression. Final code/security and architecture reviews
pass with P0=0, P1=0, P2=0, P3=0.

## Verification Evidence

- Focused locked package tests: Contracts 42, Ports 5, Store 14 (61 total).
- Full locked Rust workspace: 380/380.
- Preserved Node verification: 44/44.
- Format, strict workspace Clippy, dependency tree, forbidden driver, scoped
  I/O/SQL/credential/provider/product/website, migration inactivity,
  governance, and diff/untracked hygiene checks pass.
- Migration unchanged: SHA-256
  `7BFF021FC17F738551309C906578C8015B2DD0307D27D239C21DF1697C4D09C8`,
  Git blob `5c1bb61e220980b2087d4ec7a3c61a50a9d23ec5`.
- Independent local combined integration: `PASS`.

Review artifacts:

- `docs/reviews/WORKFLOW_AUDIT_TASK_018_2026-08-01.md`
- `docs/reviews/CODE_REVIEW_TASK_018_2026-08-01.md`
- `docs/reviews/ARCHITECTURE_REVIEW_TASK_018_2026-08-01.md`
- `docs/reviews/INTEGRATION_TASK_018_2026-08-01.md`
- `docs/workflow/WORKFLOW_LEDGER.md`

## Alignment And Enforcement Truth

TASK-018 stayed aligned with PLANS and ADR-016: it froze the physical
transaction boundary before real database work and did not duplicate domain
legality or durable truth.

| Gate | Current classification |
|---|---|
| Store contract/fake scope, hashing, atomicity, replay, faults | machine-enforced locally |
| Dependency/no-I/O/driver/migration inactivity | locally tested and scanned |
| Current-ticket/constitution governance | machine-enforced locally |
| PostgreSQL durability, restart/concurrency, roles/time/admission | missing/deferred to TASK-019+ |
| Remote Rust CI and branch protection | missing/unverified |
| Merge readiness | blocked; no committed candidate, remote, or authorization |

## Git And Scope State

- Branch: `feature/v2-rust-postgres-bootstrap`.
- HEAD: `06c3954cd941dc6b743e9e2a1d4f94c3658b3ff9`.
- Primary: `main` at `d856cf7eb7be0072aeb32316e43f0b653d73dca2`.
- Primary/V2 committed ahead/behind: `0/4`; remote/upstream: none.
- MVP-0 through TASK-018 remain one inspectable uncommitted dirty result.
- No reset, clean, branch switch, commit, push, merge, database mutation,
  publication, deployment, credential/account/payment change, deletion, or
  protected action occurred.

## Next Bounded Slice

`PLANS.md` marks `CURRENT TASK-019 GOVERNANCE`. SPEC-002 v16, ADR-017,
Postgres Store 1.1, and TASK-019 now freeze the exact-manifest PostgreSQL 17
schema/admission foundation. The bounded implementation adds a pinned
synchronous driver, an explicit administrative runner, read-only runtime
verifier, STOPPED/no-leader bootstrap, role separation, and a marker-owned
disposable 17.10 cluster. It does not add a live `ControlStore`, durable
receipt, production credential/role/database change, daemon activation,
public exposure, or unrelated website work.

## Restart Context

- Overall goal remains active through MVP-3; do not mark it complete.
- Current MVP: MVP-1 offline control core.
- Completed ticket: `docs/tickets/TASK-018-postgres-store-boundary.md`.
- Current ticket:
  `docs/tickets/TASK-019-postgres-manifest-admission-foundation.md`.
- Continue bounded reversible local work automatically; protected actions and
  primary merge remain fail-closed.

---

# Archived LATTICE DevOS TASK-017 Handoff

## Outcome

TASK-017 Gateway IPC 1.1 / wire protocol 1.0 is complete for its bounded
pure/fake MVP-1 scope. SPEC-002 AC-31 is complete. This does not complete the
live portion of AC-07, MVP-1, MVP-2, MVP-3, or the whole platform.

LATTICE remains a general local AI development platform for the user's
computer:

> OpenClaw + Rust LATTICE Core + PostgreSQL + Codex + Graphify + Hermes +
> Codebase Memory

The unrelated playmate website is not part of this repository, architecture,
tests, or roadmap.

Current milestone state:

- MVP-0 Rust foundation: complete.
- MVP-1 offline control core: in progress; TASK-017 is complete.
- MVP-2 exact-version local OpenClaw/Codex/Graphify/Hermes plus Codebase Memory:
  planned after the MVP-1 exit gate.
- MVP-3 Guardian-protected autonomy and upgrade: planned after the MVP-2
  authority/containment gate.

No routine human decision blocks TASK-018. Credentials, account/payment
changes, public exposure/publication/deployment, irreversible real effects,
security-control changes, protected release activation, and primary-branch
merge remain separately protected.

## Completed In TASK-017

### Contracts 1.7 And Ports 1.2

- Added neutral bounded peer/request/reply values for exactly Submit, Plan,
  Status, normal Approve/Reject, and task Stop. Actions derive from closed typed
  bodies; no arbitrary action, SQL, shell, path, provider, daemon, or release
  escape hatch exists.
- Bounded gateway-reused task/snapshot/attempt identifiers to 256 bytes and
  rejected all-zero authority/freshness/receipt/observation/terminal digests.
- Bound reply action, request, command, correlation, subject, page size,
  disposition, and evidence before canonical hashing.
- Changed `GatewayService` to accept server-derived peer context and a complete
  request, returning a bound reply through component-free
  `GatewayServiceError`; Rust-core failures can no longer be mislabeled as
  OpenClaw failures.

### Gateway IPC 1.1 / Wire Protocol 1.0

- Added a strict canonical JSON codec with a raw 1 MiB frame cap, depth 32,
  node 8,192, array 256, no numbers, exact fields, duplicate/unknown/version/
  action rejection, complete trailing-data checks, and redacted bounded errors.
- Added allocation-free NFC preflight for values and keys. Non-NFC request and
  reply identities, including normalization-expanding exact-bound inputs, fail
  before canonical hashing/allocation, replay insertion, or service dispatch.
- Added domain-separated request/reply digests and mechanical Task Spec 2.1
  canonical-document digest/binding verification without copying Task Domain
  semantics or exposing raw source.
- Added a pure in-memory fake client/server. Role authorization precedes replay;
  exact `(project, actor, command)` retry returns identical terminal bytes;
  changed content denies without another service call. Replay storage is capped
  at 1,024 entries while retained exact retries remain readable.
- Recovery can route only bounded Status and task Stop. ProtectedChange routes
  normally; raw `PROTECTED_RELEASE` is unrepresentable and rejected by the
  codec before service dispatch.
- Project status observes at most both the request page size and global 100-
  item cap. Stop preserves REQUESTED, ALREADY_TERMINAL, and
  RECONCILIATION_REQUIRED without claiming process interruption or completion.

### Repository Governance Repair

- `scripts/check-project.mjs` now rejects duplicate `ticket_id` values and any
  `PLANS.md` state with other than one `CURRENT TASK-nnn` marker.
- Three disposable Node regressions prove duplicate denial, multiple-marker
  denial, and the valid unique/single-marker case.
- Contracts owns neutral in-process representations and constructor-level
  identifier/cursor/page bounds. Gateway IPC owns wire layout, parser/encoded-
  frame limits, NFC enforcement, hash subjects, and replay. SPEC v14, ADR-015,
  module constitutions, routing index, ticket, and plan agree on that split.

## Review Repairs

The initial independent review found nine blockers: replay before role
authorization, zero digest sentinels, reply hashing before bounds, oversized
reused IDs, ignored request page size, unbounded replay storage, contradictory
protected semantics, incomplete reply/substitution matrices, and duplicate
TASK-017 tickets.

Final review additionally found and closed non-NFC identity/size expansion,
typed-encoder fast-fail ordering, false external component attribution for
Rust-core errors, Contracts/wire ownership drift, dependency/version drift,
and missing machine enforcement for ticket/current-marker uniqueness.

Every accepted behavioral finding received a failing regression before repair.
Final independent code/security and architecture reviews report `PASS`, with
P0=0, P1=0, P2=0, and P3=0.

## Files Added Or Materially Changed

- Gateway code/tests: `crates/lattice-gateway-ipc/**`.
- Shared interfaces/tests: `crates/lattice-contracts/{src,tests}` and
  `crates/lattice-ports/{src,tests}`.
- Workspace dependency graph: `Cargo.toml`, `Cargo.lock`, and Gateway manifest.
- Machine governance: `scripts/check-project.mjs` and
  `test/project-governance-check.test.js`.
- Governance/delivery: SPEC-002 v14, ADR-015, Gateway/Contracts/Ports and
  related module documents, TASK-017, PLANS, workflow audit, final review and
  integration reports, workflow ledger, and this handoff.

## Verification Evidence

All final local commands completed successfully:

- focused locked suites: Contracts 36, Gateway IPC 31, Ports 3 (70 total).
- `cargo test --workspace --locked`: 358 Rust tests.
- strict workspace Clippy, all targets/features, locked, `-D warnings`.
- `cargo fmt --all -- --check`.
- `npm.cmd run verify`: 41 Node tests; project check reports 221 files,
  17 constitutions, 17 unique tickets, and one current task.
- Gateway Cargo tree: only Contracts, Ports, cjson, exact serde/serde_json, and
  exact Unicode normalization plus approved transitives.
- forbidden filesystem/network/process/database/Git/provider/product and
  unrelated-website scans: zero scoped implementation matches.
- `git diff --check`: pass.
- independent code/security and architecture review: `PASS`, zero P0-P3.

Review artifacts:

- `docs/reviews/WORKFLOW_AUDIT_TASK_017_2026-08-01.md`
- `docs/reviews/CODE_REVIEW_TASK_017_2026-08-01.md`
- `docs/reviews/ARCHITECTURE_REVIEW_TASK_017_2026-08-01.md`
- `docs/reviews/INTEGRATION_TASK_017_2026-08-01.md`
- `docs/workflow/WORKFLOW_LEDGER.md`

## Alignment And Enforcement Truth

The work stayed aligned with PLANS Step 5 and the MVP-1 goal: the gateway
protocol is frozen before live transport, PostgreSQL, Codex, Graphify, Hermes,
or Codebase Memory composition.

| Gate | Current classification |
|---|---|
| Pure codec/fake action, binding, role, retry, limit, and fault behavior | machine-enforced locally |
| Project/actor/command isolation and bounded replay | machine-enforced locally |
| Unique ticket IDs and exactly one current task | machine-enforced locally |
| Dependency/no-I/O direction | locally linted and inspected |
| Contracts/wire ownership and fake/live boundary | documented plus structurally checked |
| Live OpenClaw transport, ACL, peer identity, restart, compatibility | missing/deferred under AC-07 |
| PostgreSQL durability and composed One Truth | missing/deferred |
| Remote Rust CI and branch protection | missing/unverified |
| Merge readiness | blocked; no committed candidate, remote, or authorization |

## Git And Scope State

- Branch: `feature/v2-rust-postgres-bootstrap`.
- HEAD/base: `06c3954cd941dc6b743e9e2a1d4f94c3658b3ff9`.
- Primary: `main` at `d856cf7eb7be0072aeb32316e43f0b653d73dca2`.
- Primary/V2 committed ahead/behind: `0/4`; remote/upstream: none.
- MVP-0 through TASK-017 remain one inspectable uncommitted dirty result.
- TASK-017-identifiable paths fit its final allowlist; exact shared-file
  per-ticket diff is partial/documented-only because prior V2 work is also
  uncommitted.
- No reset, clean, branch switch, commit, push, merge, installation, live
  database mutation, publication, deployment, credential/account/payment
  change, real delete, or protected action occurred.

## Incomplete And Explicitly Open

- AC-07 remains open for real OpenClaw package/schema/binary, OS-local
  transport/ACL/peer authentication, session restart/disconnect, and durable
  terminal receipt evidence.
- TASK-018 through TASK-031 remain for PostgreSQL stores, filesystem/Git/scope,
  review and Codex fakes, offline end-to-end composition, compatibility, and
  the MVP-1 exit gate.
- Exact-version live OpenClaw, Codex, Graphify, Hermes, and Codebase Memory
  remain MVP-2.
- Guardian-protected improvement, A/B activation, canary, and rollback remain
  MVP-3.

## Next Bounded Slice

`PLANS.md` marks `CURRENT TASK-018 GOVERNANCE`.

TASK-018 must freeze a typed, zero-I/O Postgres Store 1.0 boundary and
deterministic fake before any database connection:

1. re-audit TASK-017 closure and current repository/Git state;
2. define transaction request/result/error and exact project/authority/
   idempotency binding without copying domain legality;
3. keep PostgreSQL as the future sole durable writer/truth while the TASK-018
   fake remains visibly non-durable and performs no database I/O;
4. freeze migration ownership and compatibility boundaries for TASK-019;
5. update SPEC/ADR/module constitution/ticket before RED tests;
6. repeat TDD, focused/full verification, independent reviews, integration,
   ledger, and handoff.

## Restart Context

- Overall goal remains active through MVP-3; do not mark it complete.
- Current MVP: MVP-1 offline control core.
- Current marker: `CURRENT TASK-018 GOVERNANCE` in `PLANS.md`.
- Completed ticket: `docs/tickets/TASK-017-gateway-ipc.md`.
- Continue bounded, reversible local work automatically; do not introduce the
  unrelated playmate website.

---

# Archived LATTICE DevOS TASK-016 Handoff

## Outcome

TASK-016 Artifact Store 1.0 is complete for its bounded pure/fake MVP-1 scope.
SPEC-002 AC-30 is complete. This is not completion of AC-19, MVP-1, MVP-2,
MVP-3, or the full platform.

The product remains a general local AI development platform for the user's
computer:

> OpenClaw + Rust LATTICE Core + PostgreSQL + Codex + Graphify + Hermes +
> Codebase Memory

The unrelated playmate website is not part of this repository, architecture,
tests, or roadmap.

Current milestone state:

- MVP-0 Rust foundation: complete.
- MVP-1 offline control core: in progress; TASK-016 is complete.
- MVP-2 exact-version local OpenClaw/Codex/Graphify/Hermes plus Codebase Memory:
  planned after the MVP-1 exit gate.
- MVP-3 Guardian-protected autonomy and upgrade: planned after the MVP-2
  authority/containment gate.

No routine human decision blocks the next bounded local ticket. Credentials,
account/payment changes, public exposure/publication/deployment, irreversible
real effects, security-control changes, protected release activation, and
primary-branch merge remain separately protected.

## Completed In TASK-016

### Contracts 1.6

- Added neutral immutable project-scoped object/generation, provenance,
  reference, read, sweep, availability/delete, fixed owner receipt, and full
  current-head representations.
- Enforced positive signed-BIGINT-safe counters, exact SHA-256 identity,
  canonical time, runtime/producer/status/action closure, and complete binding
  validation.

### Artifact Store 1.0

- Added one public `FakeArtifactStore` owner. Lifecycle, history, quotas,
  staging, bytes, and terminal maps are composed atomically behind it; lower
  mechanisms cannot be a public second writer.
- Verified length and SHA-256 before publication. Raw bytes remain confined to
  a separately redacted in-memory fake backend and never enter retained
  requests, receipts, snapshots, checkpoints, errors, or `Debug`.
- Implemented project-isolated content generations, immutable per-use
  references, complete provenance, typed fixed-owner current authority,
  release terminality, active/suspect read lifecycle, safe delete planning,
  exact claim token, unknown-outcome reconciliation, and higher-generation
  reintroduction.
- Implemented exact applied and denied command retry before stale/time checks;
  changed content under one scoped command key rejects permanently.
- Enforced hard and configured byte/manifest/field/bundle/object/reference/
  read/staging/command/history limits at object, task, project, and store
  scopes. Task object/active-byte attribution is active-reference-only;
  retained project/store capacity and worst-case claimed/reconciliation/orphan
  capacity remain held until verified terminal evidence.
- Included holder IDs, complete persisted lifecycle strings, and the 64-byte
  domain-separated delete claim token in `FieldBytes` accounting.
- Added strict raw snapshots containing complete sanitized terminal lifecycle
  receipts. Context-free replay reconstructs lifecycle, history, quotas,
  staging, command tasks, retired scopes, and terminals, validates all
  digests/joins, and then compares an independent compact checkpoint.
- The checkpoint retains only store/limit/snapshot/replay-bound/trust-anchor
  commitments; it contains neither an owner clone, metadata row set, nor
  payload. Untrusted canonical size is preflighted before allocation, including
  control-character escape expansion.

### Review Repairs

Independent review found and closed:

- checkpoint construction that temporarily copied payload bytes;
- canonical-byte bounds checked after output allocation;
- replay that returned a trusted owner clone instead of rebuilding raw input;
- missing full lifecycle receipts needed for context-free exact retry;
- missing holder/lifecycle/claim-token `FieldBytes` projection;
- direct applied-after-replay retry evidence.

Each accepted behavioral finding received a failing regression before repair.
Final code/security and architecture re-reviews report `PASS`, with P0=0,
P1=0, P2=0, and P3=0.

## Files Added Or Materially Changed

- Workspace/contracts: `Cargo.toml`, `Cargo.lock`,
  `crates/lattice-contracts/src/lib.rs`, and Contracts tests.
- Artifact Store owner and mechanics:
  `crates/lattice-artifact-store/src/{lib,aggregate,history,quota,quota_owner,semantics,snapshot}.rs`.
- Strict context-free restore:
  `src/aggregate/snapshot_restore.rs`, `src/semantics/snapshot_restore.rs`,
  `src/snapshot_parse.rs`, `src/snapshot_contract.rs`, and
  `src/snapshot_quota.rs` under the Artifact Store crate.
- Artifact Store behavior suites under
  `crates/lattice-artifact-store/tests/`, including owner, delete, read, quota,
  staging, history, lifecycle, and replay matrices.
- Governance/delivery: SPEC-002 v12, ADR-014, Artifact Store and Contracts
  constitutions, TASK-016, PLANS, workflow audit, final code/security review,
  final architecture review, integration report, workflow ledger, and this
  handoff.

## Verification Evidence

All commands below completed successfully:

- `cargo test -p lattice-contracts --locked`: 32 tests.
- `cargo test -p lattice-artifact-store --locked`: 97 tests.
- `cargo test -p lattice-artifact-store --test artifact_owner_replay --locked`:
  8 tests.
- `cargo test --workspace --all-targets --all-features --locked`: 322 Rust
  tests.
- `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`.
- `cargo fmt --all -- --check`.
- `npm.cmd run verify`: `check=ok`, 38 preserved Node tests.
- `cargo tree -p lattice-artifact-store --edges normal --locked`: only
  Contracts, cjson, SHA-256, time, and approved transitives.
- forbidden-I/O, provider/product dependency, and unrelated-website scans:
  zero scoped implementation/dependency matches.
- payload fixture containment assertions: raw snapshot, checkpoint, and owner
  debug contain no fixture bytes; replayed verified read reports `MissingBytes`.
- `git diff --check`: pass.

Verification artifacts:

- `docs/reviews/WORKFLOW_AUDIT_TASK_016_2026-07-30.md`
- `docs/reviews/CODE_REVIEW_TASK_016_2026-07-30.md`
- `docs/reviews/ARCHITECTURE_REVIEW_TASK_016_2026-07-30.md`
- `docs/reviews/INTEGRATION_TASK_016_2026-07-30.md`
- `docs/workflow/WORKFLOW_LEDGER.md`

## Workflow Ledger

| Stage | Status | Evidence |
|---|---|---|
| Inspect repository/Git | valid | workflow audit, branch/base/status |
| Clarify decisions | valid | SPEC v12 and ADR-014 |
| Specification | valid | AC-30 complete; durable/live AC-19 remains open |
| Module governance | valid | Artifact Store 1.0 and Contracts 1.6 |
| Ticket decomposition | valid | bounded non-parallel TASK-016 |
| Branch/worktree plan | valid | dirty V2 worktree preserved; V1 untouched |
| TDD implementation | valid | RED/GREEN behavior and review regressions |
| Focused/full verification | valid | 32 Contracts, 97 Artifact, 322 Rust, 38 Node |
| Code/security review | pass | zero remaining P0-P3 |
| Architecture review | pass | zero P0-P3; no amendment |
| Local integration | pass/partial | combined result passes; no committed candidate |
| Remote CI/merge | missing/blocked | no remote, CI, branch protection, candidate, or authorization |

## Alignment And Enforcement Truth

The work stayed aligned with PLANS Step 5 and the overall MVP-1 goal: one pure
artifact authority was frozen before PostgreSQL, filesystem effects, OpenClaw,
Codex, Graphify, Hermes, or Codebase Memory consumes it.

| Gate | Current classification |
|---|---|
| Pure artifact authority/quota/retry/replay/checkpoint behavior | machine-enforced locally |
| Project isolation and fixed owner/runtime contracts | machine-enforced locally |
| Dependency/no-I/O direction | locally linted and inspected |
| Governance semantics and ownership | documented plus structurally checked |
| PostgreSQL transactions/durability/restart | missing/deferred under AC-19 |
| Real filesystem containment/staging/delete | missing/deferred |
| Live provider/authority authentication | missing/deferred |
| One Gateway/One Truth/One Writer at composed runtime | documented-only in this slice |
| Remote Rust CI and branch protection | missing/unverified |
| Merge readiness | blocked; no committed candidate or authorization |

## Git And Scope State

- Repository:
  `C:\Users\f7212\Documents\Codex\2026-07-29\files-mentioned-by-the-user-2026\outputs\lattice-devos-v2`
- Branch: `feature/v2-rust-postgres-bootstrap`
- HEAD/base: `06c3954cd941dc6b743e9e2a1d4f94c3658b3ff9`
- Primary branch: `main` at `d856cf7eb7be0072aeb32316e43f0b653d73dca2`
- Primary/V2 committed ahead/behind: `0/4`
- Remote/upstream: none.
- The shared MVP-0 through TASK-016 result remains uncommitted and dirty.
- Identifiable TASK-016 paths fit its allowlist; exact per-ticket shared-file
  scope is `partial/documented-only` because prior V2 work is also uncommitted.
- No reset, clean, branch switch, commit, push, merge, installation,
  publication, deployment, credential/account/payment change, PostgreSQL
  mutation, real delete, or protected action occurred.

## Incomplete And Explicitly Open

- AC-19 remains open for PostgreSQL metadata/reference transactions,
  serialization, durability, restart, and same-transaction effect admission.
- Real filesystem staging/flush/rename/link containment/read/delete and orphan
  reconciliation remain open.
- Fake OpenClaw IPC, remaining PostgreSQL stores, workspace/scope enforcement,
  fake Codex/reviewer, offline end-to-end orchestration, and the MVP-1 exit gate
  remain incomplete.
- Exact-version OpenClaw, Codex, Graphify, Hermes, and Codebase Memory remain
  MVP-2.
- Guardian-protected improvement/activation/rollback remains MVP-3.

## Next Bounded Slice

At TASK-016 closure, `PLANS.md` then marked `CURRENT TASK-017 GOVERNANCE`;
TASK-017 is now complete as recorded at the top of this file.

TASK-017 should freeze and implement a pure/fake OpenClaw IPC boundary without
installing or invoking OpenClaw:

1. re-audit TASK-016 closure and confirm the slice still serves MVP-1;
2. define the only normal typed gateway actions for task submission, status,
   approval routing, and stop;
3. keep the CLI as a recovery/test client over the same contract, not another
   normal gateway;
4. ensure IPC grants no direct PostgreSQL/Git/provider/credential/protected-
   release authority and cannot own a Codex writer thread;
5. update SPEC/ADR/module constitution/ticket before implementation;
6. repeat TDD, focused/full verification, independent reviews, integration
   report, ledger, and handoff.

## Restart Context

- Overall goal remains active through MVP-3; do not mark it complete.
- Current MVP: MVP-1 offline control core.
- Historical next marker at TASK-016 closure: `CURRENT TASK-017 GOVERNANCE`.
- Completed ticket: `docs/tickets/TASK-016-artifact-store.md`.
- First checks:
  - `git status --short --branch`
  - `git rev-parse HEAD`
  - `cargo test --workspace --all-targets --all-features --locked`
  - `npm.cmd run verify`
- Continue bounded, reversible local work automatically.
- Do not introduce the unrelated playmate website.
