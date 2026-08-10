# Codex 新窗口提示詞 — Connectivity First / Governance v2

把以下整段貼入新的 Codex 窗口。

---

你正在接手私有 GitHub Repository：

```text
z72124223/lattice-devos
```

## 一、先驗證遠端，不要假設

先執行唯讀檢查：

1. 讀取 `AGENTS.md`。
2. 讀取：
   - `docs/adr/ADR-024-connectivity-first-microkernel-and-mcp-providers.md`
   - `docs/governance/DEVELOPMENT_GOVERNANCE_V1.md`
   - `docs/modules/task-domain/MODULE_CONSTITUTION.md`
   - `docs/modules/lattice-ports/MODULE_CONSTITUTION.md`
   - `docs/modules/latticed/MODULE_CONSTITUTION.md`
   - `docs/tickets/GH-9-reflection-evolution.md`
   - GitHub Issues #4、#5、#9、#10 及最新留言
3. `git fetch --all --prune`。
4. 檢查：
   - current branch / HEAD / upstream
   - `git status --short`
   - remote branch HEAD
   - ahead / behind
   - 相關 PR、CI、Branch Protection
5. 預期治理分支：

```text
docs/governance-connectivity-first-v2
```

6. 冻結的 GH-9 程式基線必須仍是：

```text
feature/gh-9-reflection-evolution
a86747ddfa182590c49324a1add15ce4bdc70978
```

若遠端狀態與以上不一致，立即停止修改，回報固定欄位，不自行 rebase、merge、reset、force-push 或猜測新基準。

## 二、當前最高優先級

目前產品 P0 是：

> **真實 ChatGPT ↔ LATTICE 的來回通訊、受控提交與跨 Session durable status。**

不是 GH-9 Live PostgreSQL，不是 TASK-037 Formal，不是 Hermes 生產執行，也不是新增更多 Runtime 功能。

必須分開判定以下 Gate：

```text
DISCOVERY_VERIFIED
READ_ROUNDTRIP_VERIFIED
BOUNDED_SUBMIT_VERIFIED
CROSS_SESSION_DURABLE_READ_VERIFIED
CONTROL_VERIFIED
```

不得把下列證據互相冒充：

```text
Tunnel health != ChatGPT invocation
Tools/list != Tool call
Local MCP invocation != ChatGPT invocation
Same session status != Cross-session durable replay
Local tests != Remote CI
```

## 三、硬性禁止

在使用者另行放行前：

- 不修改 frozen SHA `a86747dd…`。
- 不啟動 TASK-037 Formal。
- 不啟動 GH-9 Live PostgreSQL Replay。
- 不新增 Reflection Runtime 功能。
- 不啟動真實 Hermes。
- 不建立 Global Reflection Queue、`claim_next`、Unattended Worker。
- 不新增 Crate、MCP Server、Schema、Migration、Queue、Truth Source、Writer 或認證系統。
- 不提供任意 Shell、SQL、Path、Git Command、Credential、Provider URL、Prompt、Lease、Fence 或 Writable Thread 給 GPT。
- 不建立 PR、不 Merge、不 Deploy、不 Release、不變更 Default Branch。
- 不因 ChatGPT 方案或平台限制而弱化 LATTICE 安全邊界。

## 四、第一階段：只做現況查證與最短路徑設計

先回答以下問題，並為每項標記：

```text
DECLARED
LOCAL_VERIFIED
REMOTE_VERIFIED
CI_VERIFIED
LIVE_VERIFIED
UNKNOWN
```

### A. 現有 ChatGPT / MCP 連線證據

確認：

1. 現行 `latticed` 是否仍只公開四個工具：
   - `lattice_delivery_run`
   - `lattice_delivery_status`
   - `lattice_task_submit`
   - `lattice_task_status`
2. 目前 ChatGPT App / Secure MCP Tunnel 使用的是哪個 Connector Snapshot。
3. 真實 ChatGPT 是否已看到四個工具，還是只有歷史兩工具證據。
4. 是否曾從真實 ChatGPT 成功呼叫 read-only tool。
5. 是否曾從真實 ChatGPT 成功呼叫 `lattice_task_submit`。
6. 是否曾由另一個新 ChatGPT request/session 讀到同一 PostgreSQL-backed status。
7. 哪些只有 local canonical acceptance，哪些是 real ChatGPT evidence。
8. 當前 ChatGPT workspace / plan / connector policy 是否允許 write action；只接受官方或可直接觀察的證據，不靠猜測。

### B. 最小測試路徑

提出最短、最安全的測試順序：

```text
1. Real ChatGPT discovery refresh
2. Real read-only round-trip
3. Fixed bounded CONTROLLED_CODEX_CANARY submit
4. Separate new ChatGPT session durable status read
```

優先重用現有 Tunnel、Profile、四工具 Contract 與 Canary，不先修改 Rust。

如果平台阻擋 write，固定回報：

```text
PLATFORM_CAPABILITY_BLOCKED
```

並說明：

- 阻擋發生在哪個 Gate；
- 官方／直接證據；
- LATTICE 本身是否仍可 read-only 使用；
- 可比較的方案：支援 Full MCP 的 Workspace、OpenAI API Gateway、GitHub Issue Bridge、現有本機受控入口；
- 不自行選擇付費、帳號或外部承諾方案。

## 五、第二階段：只有具體 blocker 才准修改

完成現況查證後：

- 若現有系統已足夠，只產生精確操作步驟與證據模板，不改程式。
- 若發現文件或 Connector Snapshot stale，只做最小文件／設定修正，先說明影響。
- 若發現真實的 LATTICE transport / schema / adapter bug，先建立：
  - exact first failure；
  - 最小 RED；
  - 影響範圍；
  - simpler alternatives；
  - complexity justification。
- 在未證明需要前，不得新增 Process、Protocol、Crate、MCP Server、Schema 或 Queue。

架構原則：

```text
Microkernel keeps authority
Port first
MCP adapter second
Provider never becomes truth / policy / writer / scheduler
```

## 六、Issue 重複與權威

當發現舊 Issue、文件或驗收條件重複／衝突時，必須標記：

```text
CANONICAL
DUPLICATE_OF
OVERLAPS_WITH
SUPERSEDES
SUPERSEDED_BY
SPLIT_FROM
MERGED_INTO
HISTORICAL
```

目前權威關係：

- Issue #4：ChatGPT MCP 連線與 bounded gateway。
- Issue #5：能力導向 Provider、替換、備援與退役。
- Issue #9：Reflection、失敗學習、非同步 Hermes；它取代普通核心 Task 必須同步等待 Hermes 的舊要求。
- Issue #10：熔斷、分段驗收、局部失效與治理。

遇到衝突不得自行選一份；建立 reconciliation note 並回報。

## 七、每次 Push 的 Checkpoint Receipt

任何遠端 Push 必須依
`docs/governance/DEVELOPMENT_GOVERNANCE_V1.md` 產生
`lattice.checkpoint-receipt/v1`，至少包含：

- canonical Issue / relationship
- branch / base / parent / commit / tree SHA
- development start/end / Asia-Taipei / monotonic elapsed
- exact changed paths
- 每個 check 的 command、exit、evidence level、digest/path
- live components exercised
- explicit NOT_TESTED
- blockers
- remote SHA equality / ahead-behind / worktree clean
- CI / branch protection / PR / merge / deploy / release
- highest justified evidence level

不得只說「測試通過」或「已推上 GitHub」。沒有 Exact SHA 的報告視為 `DECLARED`。

## 八、第一次回報格式

完成唯讀查證後，先回報，不要立刻開始長工作：

```text
REMOTE_BRANCH=
REMOTE_SHA=
LOCAL_SHA=
AHEAD_BEHIND=
WORKTREE=
RELATED_PR=
REMOTE_CI=
BRANCH_PROTECTION=

DISCOVERY_GATE=
READ_ROUNDTRIP_GATE=
BOUNDED_SUBMIT_GATE=
CROSS_SESSION_DURABLE_READ_GATE=
CONTROL_GATE=

CURRENT_FIRST_BLOCKER=
BLOCKER_OWNER=LATTICE|CHATGPT_PLATFORM|TUNNEL|UNKNOWN
RUST_CHANGE_REQUIRED=YES|NO|UNPROVEN
SIMPLEST_NEXT_ACTION=

GH9_LIVE_POSTGRES=PAUSED_FOR_ARCHITECTURE_REVIEW
TASK037_FORMAL=PROHIBITED
FROZEN_GH9_SHA=a86747ddfa182590c49324a1add15ce4bdc70978
```

使用簡單中文說明：目前真正成功到哪一層、哪一層尚未證明、下一步為什麼是最短且安全的路徑。

---

不要把「做更多」誤認成「進度」。本窗口的成功標準是：

> **以最少變更，對真實 ChatGPT ↔ LATTICE 邊界取得可採信的明確 PASS 或明確平台 BLOCKED。**
