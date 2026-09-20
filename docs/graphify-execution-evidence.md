# Graphify 執行方式與證據

本頁回答：要求 Codex 使用 LATTICE，是否代表每個任務一定執行 Graphify？**目前沒有這項保證。**

查核日期：2026-09-20。以下原碼與行號固定於
[`b786408fb0b9e3772596dca4120d6bf58d043f53`](https://github.com/z72124223/lattice-devos/tree/b786408fb0b9e3772596dca4120d6bf58d043f53)。
這是包含 Windows 候選安裝程式的開發分支快照；不是宣稱預設分支、所有下載包或已安裝執行檔都具有相同內容。
本次是原碼與既有測試內容查核，沒有新增或執行動態任務測試。發布本說明不代表功能修復或安裝候選版轉正式。

## 1. 文字規則、模式選擇與硬性關卡

| 機制 | 實際作用 | 不能據此宣稱 |
|---|---|---|
| Codex 的 `AGENTS.md` | 要求模型先查 LATTICE 狀態、核對或登記工作 | 每個任務必定查圖譜，或未查圖譜就無法動工 |
| `LATTICE_RUNTIME_INTEGRATION="GRAPHIFY"` | 選擇 Graphify 整合模式 | 每次工具呼叫都啟動 Graphify，或自動把圖譜加入模型上下文 |
| 任務提交前的程式關卡 | 必須實際驗證圖譜查詢證據、缺少時拒絕任務，才構成這種保證 | 已查核的一般任務提交路徑沒有這項要求 |

安裝程式雖使用 `install_global_hook` 這個名稱，實際寫入的是 `AGENTS.md`：
[scripts/lattice-one-click-install.py:142–153](https://github.com/z72124223/lattice-devos/blob/b786408fb0b9e3772596dca4120d6bf58d043f53/scripts/lattice-one-click-install.py#L142-L153)。

```python
def install_global_hook(codex_home: Path, *, state: Path | None = None,
                        bundle: Path | None = None) -> dict:
    """Manage only our global guidance block; retain all other user instructions."""
    path = codex_home / "AGENTS.md"
```

同檔 [193–198 行](https://github.com/z72124223/lattice-devos/blob/b786408fb0b9e3772596dca4120d6bf58d043f53/scripts/lattice-one-click-install.py#L193-L198)
明確要求先提交真正工作，然後「任務成功登記後，在需要程式關係資料時」才更新圖譜。
相同專案、相同 commit 已有可讀關係時不要重建。
這是安裝模板的行為；個別使用者是否安裝、修改或載入這些規則，仍須查其本機設定。

## 2. 真正工作中的三條路徑

| 入口 | 工作 | 證據 |
|---|---|---|
| `lattice_task_submit`／`lattice_task_status` | 提交或讀回正式任務；不是關係查詢的別名 | [MCP 分派 2855–2862](https://github.com/z72124223/lattice-devos/blob/b786408fb0b9e3772596dca4120d6bf58d043f53/apps/lattice-runtime/src/mcp.rs#L2855-L2862)；[一般提交 resolve → admit → schedule，9248–9260](https://github.com/z72124223/lattice-devos/blob/b786408fb0b9e3772596dca4120d6bf58d043f53/apps/lattice-runtime/src/composition.rs#L9248-L9260) |
| `graphify-refresh` | 對指定、已提交的乾淨 Git 版本建立圖譜；已有相同來源收據時直接重用，否則執行分析並保存圖譜與收據 | [refresh 入口及收據重用 12136–12175](https://github.com/z72124223/lattice-devos/blob/b786408fb0b9e3772596dca4120d6bf58d043f53/apps/lattice-runtime/src/composition.rs#L12136-L12175)；[Graphify adapter 與 orchestrator，12463–12509](https://github.com/z72124223/lattice-devos/blob/b786408fb0b9e3772596dca4120d6bf58d043f53/apps/lattice-runtime/src/composition.rs#L12463-L12509) |
| `lattice_code_relations` | 按專案及精確 commit 讀取 PostgreSQL 已保存的關係；無分析收據則報錯，不會自行補跑分析 | [實作 9365–9475](https://github.com/z72124223/lattice-devos/blob/b786408fb0b9e3772596dca4120d6bf58d043f53/apps/lattice-runtime/src/composition.rs#L9365-L9475)；[SQL 讀取 156–205](https://github.com/z72124223/lattice-devos/blob/b786408fb0b9e3772596dca4120d6bf58d043f53/crates/lattice-postgres-store/src/control_product.rs#L156-L205) |

MCP 的實際分派：

```rust
ToolOperation::TaskSubmit(arguments) => {
    closed_task_public_status(self.service.task_submit(&arguments))
}
ToolOperation::TaskStatus(arguments) => {
    closed_task_public_status(self.service.task_status(&arguments))
}
ToolOperation::ControlSnapshot(arguments) => self.service.control_snapshot(&arguments),
ToolOperation::CodeRelations(arguments) => self.service.code_relations(&arguments),
```

分派各自獨立，不足以單獨證明所有下游效果；必須繼續追服務實作。
本次追到的關係查詢確實呼叫資料庫，而非重新啟動 Graphify 分析：

```rust
// composition.rs:9447–9452
let receipt = receipt
    .ok_or_else(|| ToolExecutionError::new("CODE_RELATIONS_SOURCE_RECEIPT_UNAVAILABLE"))?;
let mut product = connect_control_product(&core)?;
let mut page = product
    .code_relations(&receipt, &arguments.query, arguments.limit)
    .map_err(ToolExecutionError::new)?;
```

底層執行 `SELECT control_product.code_relations_v1($1,$2,$3,$4,$5,$6,$7)`。
因此「讀取 Graphify 產生的資料」與「啟動 Graphify 分析程序」須分開計算。

## 3. CORE_ONLY 是否攔截真正任務中的關係查詢？

在以下三個完整檔案內，**沒查到**根據 `LATTICE_RUNTIME_INTEGRATION`、`CORE_ONLY` 或 `GRAPHIFY`
決定跳過查詢的邏輯：

| 原始碼 | 查到的實作 |
|---|---|
| [apps/lattice-runtime/src/code_relations.rs:105–144](https://github.com/z72124223/lattice-devos/blob/b786408fb0b9e3772596dca4120d6bf58d043f53/apps/lattice-runtime/src/code_relations.rs#L105-L144) | 查詢參數驗證；同檔另有結果完整性驗證，沒有 Graphify 分析啟動入口 |
| [apps/lattice-runtime/src/mcp.rs:2783–2793](https://github.com/z72124223/lattice-devos/blob/b786408fb0b9e3772596dca4120d6bf58d043f53/apps/lattice-runtime/src/mcp.rs#L2783-L2793) | 驗證參數後形成 `CodeRelations` 操作，再於 2862 行呼叫服務 |
| [apps/lattice-runtime/src/task_control.rs:575–594](https://github.com/z72124223/lattice-devos/blob/b786408fb0b9e3772596dca4120d6bf58d043f53/apps/lattice-runtime/src/task_control.rs#L575-L594) | `execute_command` 寫任務帳本，沒有直接呼叫 Graphify |

在上述固定版本執行以下搜尋，零筆命中，`rg` 結束碼為 `1`：

```sh
rg -n 'LATTICE_RUNTIME_INTEGRATION|CORE_ONLY|GRAPHIFY|RuntimeIntegration|integration_mode|uses_graphify' apps/lattice-runtime/src/code_relations.rs apps/lattice-runtime/src/mcp.rs apps/lattice-runtime/src/task_control.rs
```

繼續追到 `composition.rs:9365–9475` 的真正 `code_relations` 實作，也沒有模式攔截。
只要其他來源與資料庫條件成立，不能用 `CORE_ONLY` 當作「這條查詢一定不會讀圖譜」的保證。
這也不代表每個任務必定啟動 Graphify 分析。

別的路徑確實有模式判斷：
[run_task_downstream_json:5532–5540](https://github.com/z72124223/lattice-devos/blob/b786408fb0b9e3772596dca4120d6bf58d043f53/apps/lattice-runtime/src/composition.rs#L5532-L5540)。
但它由 [相容 delivery-run 入口 8828–8854](https://github.com/z72124223/lattice-devos/blob/b786408fb0b9e3772596dca4120d6bf58d043f53/apps/lattice-runtime/src/composition.rs#L8828-L8854)呼叫，
不能當作一般 `task_submit`／`code_relations` 均有攔截的證據。

目前[環境變數解析 5123–5140](https://github.com/z72124223/lattice-devos/blob/b786408fb0b9e3772596dca4120d6bf58d043f53/apps/lattice-runtime/src/composition.rs#L5123-L5140)
接受 `CORE_ONLY`、`GRAPHIFY`；未設定時為 `CORE_ONLY`。`FULL_CHAIN` 與 `GRAPHIFY_HERMES` 都被拒絕。
Hermes 已退休；歷史型別或文件出現舊名稱，不代表有可啟用的正式產品模式。

## 4. 現有測試能證明什麼？

**沒查到「以 CORE_ONLY 執行真正任務，再觀測 Graphify 函式呼叫次數為零」的對應測試。**
搜尋範圍包括上述三檔的單元測試、`apps/lattice-runtime/tests/**/*.rs`，以及 `composition.rs` 的任務／關係查詢相關測試。

| 現有測試 | 實際驗證 | 不足之處 |
|---|---|---|
| [code_relations.rs:159–220](https://github.com/z72124223/lattice-devos/blob/b786408fb0b9e3772596dca4120d6bf58d043f53/apps/lattice-runtime/src/code_relations.rs#L159-L220) | 固定 JSON 的資料完整性、查詢參數與範圍 | 不執行真正任務，不觀測 Graphify 呼叫 |
| [tests/mcp.rs:1409–1433](https://github.com/z72124223/lattice-devos/blob/b786408fb0b9e3772596dca4120d6bf58d043f53/apps/lattice-runtime/tests/mcp.rs#L1409-L1433) | MCP 關係查詢參數及協定分派 | `FakeService` 沒有實作真實關係查詢，預期 `CODE_RELATIONS_UNAVAILABLE`；不會到 production SQL 路徑 |
| [tests/mcp.rs:665 起](https://github.com/z72124223/lattice-devos/blob/b786408fb0b9e3772596dca4120d6bf58d043f53/apps/lattice-runtime/tests/mcp.rs#L665) | task-tool 測試禁止錯誤分派到 delivery run | panic 攔的是 delivery 方法，不是 Graphify 分析或圖譜查詢函式 |

若要宣稱零呼叫，仍需在真正任務路徑執行測試，對 Graphify 分析入口及圖譜查詢入口分別設置可觀測的計數器或禁止呼叫的替身，
並以 `CORE_ONLY`／`GRAPHIFY` 對照，驗證測試確實能偵測到呼叫。這是缺少的證據要求，不是本次已完成的驗收。

`runtime-health` 是獨立健康檢查；`receipt-state` 讀交付收據。
它們成功、回傳 `PREPARED`／`DEFERRED`，或安裝驗收成功，都不能代替上述真正任務路徑的零呼叫證據。
讀回既存圖譜也不能證明本次任務曾重新執行分析。

安裝程式確實有另一項硬性驗收：要求 refresh 回傳 `PERSISTED`，再透過 MCP 讀回，
見 [scripts/lattice-one-click-install.py:433–451](https://github.com/z72124223/lattice-devos/blob/b786408fb0b9e3772596dca4120d6bf58d043f53/scripts/lattice-one-click-install.py#L433-L451)。
它驗證的是該次安裝流程，不是所有後續任務都必須查圖譜。

## 5. 上下文用量與版本追溯

`code_relations` 的 [limit 為 1–32](https://github.com/z72124223/lattice-devos/blob/b786408fb0b9e3772596dca4120d6bf58d043f53/apps/lattice-runtime/src/code_relations.rs#L134-L136)，
並有 [750,000 位元組的內層 JSON 序列化 UTF-8 長度上限](https://github.com/z72124223/lattice-devos/blob/b786408fb0b9e3772596dca4120d6bf58d043f53/apps/lattice-runtime/src/composition.rs#L9469-L9473)。
[MCP 包裝 4112–4117](https://github.com/z72124223/lattice-devos/blob/b786408fb0b9e3772596dca4120d6bf58d043f53/apps/lattice-runtime/src/mcp.rs#L4112-L4117)
另將內容放入 `content.text` 與 `structuredContent`，所以這不是完整傳輸回應的大小上限。
這些邊界檢查不能當成實際 token 用量、平均回應大小或「不會造成上下文負擔」的測量結果。
任務沒有固定查圖譜的關卡，也不代表額外開銷必為零；應分別量測分析次數、查詢次數、回應位元組與模型實際收到的內容。
本次沒有取得這組動態測量。

可直接閱讀單筆 Git 提交與差異，無須依賴 commits 清單頁：

- [2aa47b4：建立 CORE_ONLY Runtime](https://github.com/z72124223/lattice-devos/commit/2aa47b4c4ee7e59637fabb6b84a36da8086aba2f)
- [59fadc0：獨立 Runtime 健康檢查](https://github.com/z72124223/lattice-devos/commit/59fadc0d9e2994710f6c9099dc18ef804b558216)
- [88bd17c：加入收據綁定的唯讀程式關係查詢](https://github.com/z72124223/lattice-devos/commit/88bd17cf04ec58f06f3d40ca3012eac170e952f8)
- [9f3984c：移除正式 Runtime 的 Hermes 反思](https://github.com/z72124223/lattice-devos/commit/9f3984c39aeda65621a930715f0cf1658c78da66)

以上是分次演進的程式，不是一次更新就證明所有路徑與宣稱均已驗收。

## 6. 後續實作：由 Runtime 產生使用紀錄

前面各節保留固定 commit 的歷史查核。以下是本版新增行為，不回填或推測舊任務的使用紀錄。

- [Runtime 觀測](../apps/lattice-runtime/src/graph_usage.rs)：操作前先保存 START，操作後保存 FINISH。FINISH 失敗不回成功；程序中斷留下待完成紀錄。
- [分析函式入口](../crates/lattice-graphify-adapter/src/ports.rs) 與 [查詢函式入口](../crates/lattice-postgres-store/src/control_product.rs) 各自計數；同步 Runtime 操作讀取同執行緒的前後差值。計數包含進入函式後的失敗，不等於子程序成功啟動。
- [真實工具與 refresh 流程](../apps/lattice-runtime/src/composition.rs) 將結果寫入 [獨立的 PostgreSQL 表](../db/extensions/control-product/graph-usage-v1.sql)。原 `code_relations_v1` SQL 查詢維持唯讀；整體工具呼叫會另寫稽核紀錄。
- [MCP](../apps/lattice-runtime/src/mcp.rs) 新增 `lattice_graph_usage`；可依專案、或專案加任務讀回全量統計及最近 20 筆。讀使用紀錄本身不新增紀錄。

使用方式：對真正提交的任務，將回傳的 `task_ref` 同時傳給 `lattice_code_relations`，或傳給
customer launcher 的 `graphify-refresh --project-id <project_id> --task-ref <task_ref>`。
Runtime/CLI 底層入口為 `latticed --graphify-refresh-project <project_id> --task-ref <task_ref>`。
任務必須存在且屬於該專案，PostgreSQL 會驗證；不自動猜測目前任務。舊呼叫省略 `task_ref` 仍可執行，但顯示 `UNBOUND`。

| 欄位或結果 | 可證明的範圍 |
|---|---|
| `analysis_calls` / `query_calls` | 本次受觀測操作內，實際分析／圖譜查詢函式進入次數；不代表模型理解或採用資料 |
| `ANALYZED` / `REUSED` / `QUERIED` / `FAILED` | 新分析、重用既存分析、查詢或失敗；重用不增加分析次數 |
| `query_records` | 成功查詢實際回傳筆數的合計；不是整個圖譜大小 |
| `result_bytes` / `measured_results` | 有量測的內層結果 JSON UTF-8 位元組合計／結果數；不含 MCP/JSON-RPC 包裝，也不是 token 數；失敗未量測結果大小 |
| `duration_ms` | 已完成觀測的耗時合計；不含 START 寫入及 FINISH 落盤時間 |
| `UNKNOWN` | 沒有觀測紀錄；不能宣稱沒使用過 Graphify |
| `INCOMPLETE` | 有 START 沒有 FINISH；已知統計不完整 |
| `OBSERVED_CALLS_ONLY` | 已記錄的呼叫完成；仍不代表整個 Codex 對話或所有外部程式都被監測 |

重送同一筆資料庫寫入以 `usage_id` 保持冪等；重新呼叫工具是另一個真實操作，會產生新的 `usage_id`。
建立 START 前的參數／專案／來源設定拒絕不列入操作統計。沒有憑證、完整查詢文字或完整圖譜寫入此紀錄，查詢只保存摘要。

這項變更沒有增加「未查 Graphify 就不准完成任務」的閘門，也沒有將 `CORE_ONLY` 改成全域禁止 Graphify 的開關。
它不涵蓋健康檢查、舊 delivery 路徑或任意外部 Graphify 呼叫；不能用此摘要證明 CORE_ONLY 任務全程零呼叫。

驗收腳本 [verify-lattice-graph-usage.py](../scripts/verify-lattice-graph-usage.py) 在獨立 PostgreSQL 中測試任務綁定、重播、失敗、權限與重啟讀回。
測試範圍以腳本實際輸出的 scope/status 為準；SQL 測試寫入的 ANALYZED／REUSED 範例不是 Graphify 成功分析證據。
新版原始碼、已安裝 Runtime 與 GitHub 下載包是不同交付狀態；須經更新及 `--postgres-bootstrap` 後才有新資料表，不能以此文件視為安裝包已發布。

本地驗收（2026-09-20）：MCP 51 項、參數解析 3 項、資料庫使用紀錄單元測試 3 項、查詢計數器 2 項、
真分析介面失敗呼叫計數 1 項、customer launcher 31 項、備份 10 項及安裝流程 38 項通過。
隔離 PostgreSQL 的 20 項 SQL 檢查、舊結構升級與真 MCP 失敗／重試／程序重啟讀回均通過。
`composition::tests::graph_usage_records_real_relation_entry_and_sql_when_provisioned` 明確執行後通過：
未進查詢入口 0 次、入口拒絕 1 次、成功 SQL 1 次；固定測試圖譜回傳 1 筆、672 bytes，與使用紀錄一致，PostgreSQL 重啟後摘要不變。
該測試以合成圖譜驗證真資料庫與真函式入口，沒有執行正式 Graphify 新分析；也不是完整 Codex 任務零呼叫驗收。
`cargo build`、`npm run check` 通過；嚴格 `clippy -D warnings` 仍受既有 lint 錯誤阻擋，未宣稱全面靜態檢查通過。
