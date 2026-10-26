# LH-V1-E2E-01 本機受控驗收

正式工作：`82662fd7a130df6d5e7c1e8569764758248a5f7f94ea8e2f635bd5b1087e1227`。
執行 claim：`559f0d7f-081f-461f-bd57-5ea59fd28dfc`；input：`LH-V1-E2E-01-attempt4-20261003`。
本次 turn：`01a10208-cdd3-74a3-9705-bf8258e59bdf`。

首次失敗與修復後成功，均以預設 PowerShell／login 執行完全相同的獨立命令：

```powershell
node 'C:\Users\f7212\AppData\Local\LATTICE\runtime-delivery\control-worktrees\82662fd7a130df6d5e7c1e8569764758248a5f7f94ea8e2f635bd5b1087e1227\lh-v1-e2e-fixture\entry.mjs'
```

重跑唯讀驗收：

```powershell
node --test 'C:\Users\f7212\AppData\Local\LATTICE\runtime-delivery\control-worktrees\82662fd7a130df6d5e7c1e8569764758248a5f7f94ea8e2f635bd5b1087e1227\lh-v1-e2e-fixture\acceptance.test.mjs'
```

唯一修復是於診斷收據與 Control HTTP 200 之後新增 `message.mjs`，內容為
`export const message = 'LH_V1_E2E_OK';` 加換行。`entry.mjs` 在首次失敗後未改動；
目前位元組 SHA-256 與準備時保存的 `96ee21ee2949c7a833d6f948879820a2b181c88d53036ef092ea85c2cc869abb` 相同。

控制器的 `observed-chain.json` 保存以下原生事件；`native-failure.json`、`prepare-input.json`、
`prepare-command.txt`、`control-binding.json` 也由控制器提供。只有接受的準備 CLI 建立 `receipt/evidence.json` 與 `receipt/request.json`。

| 順序 | 原生事件 ID | 實際結果 |
|---|---|---|
| 1 失敗 | `exec-6f003e5c-b34a-4cdf-a4b7-65860d6423d2` | `ERR_MODULE_NOT_FOUND`，exit 1，缺少 `./message.mjs` |
| 2 準備 | `exec-b73f2643-85a5-4714-b520-9afd2aefbf5e` | `prepared`，exit 0，回傳獨立診斷命令 |
| 3 診斷 | `exec-d068fd59-0380-4797-8ed3-a21c077f8b14` | 僅原始收據 JSON，`selected`，exit 0 |
| 4 成功 | `exec-cf4e64b7-1526-4ebb-b212-89e6021c1bf4` | 同命令 exit 0，stdout 精確為 `LH_V1_E2E_OK\n` |

Control 在 `2026-10-03T13:52:55.332Z` 記錄 HTTP 200，早於收據的
`validUntil=2026-10-03T13:56:13.935Z`；原生工作綁定 `nativeClaimBindingVerified=true`。
`producerVerified`、`inputFileBytesVerified`、`diagnosticSemanticsVerified` 均為 false。
準備／診斷保留 `advisoryOnly=true`、`adopted=false`，`nativeProvenanceVerified` 與
`authorizationVerified` 均為 false；來源與 importer 位元組驗證為 true。修復權限來自本工作明示授權。
成功事件鏈於 `2026-10-03T13:53:25.665Z` 保存。

測試使用 Node 標準函式庫，核對目前入口、原始碼與收據雜湊、事件次序與命令、相同 task／claim／thread／turn、
HTTP 及旗標，並以記憶體中的竄改副本驗證拒絕行為。缺模組反例使用 Node 的靜態相對 import 與不存在的獨立名稱，
不刪除修復檔，也不建立其他檔案。重跑不需要實驗旗標、網路、安裝或寫入，不會重跑準備或刷新歷史有效期限。

本次 focused test 已執行一次：exit 0，6 項通過、0 項失敗、0 項略過，Node 回報總測試時間 225.4421 ms。
未執行舊產品測試套件；此結果僅證明上述本機行為及保存證據的一致性。

這是 Windows／Node 本機受控驗收。檔案驗證能核對控制器保存的事件與結果，但不自行證明控制器來源或作業系統隔離；
事件順序依控制器的 orderedItemIds，原生事件物件本身沒有逐筆時間戳。
未宣稱產品效益、自動修復、Jev 採用或發布，也未修改正式完成狀態。
獨立驗收與 Runtime 固定 Node 測試／匯入由既有服務在執行者 final 後處理，PostgreSQL 正式完成與讀回尚未由本執行者驗證。
