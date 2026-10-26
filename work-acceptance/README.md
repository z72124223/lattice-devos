# 工作樹唯讀驗收

本目錄是既有 LATTICE 工作的驗收產物，使用 Node.js 標準函式庫，不需安裝套件。沒有新增網站、伺服器或產品功能，不更新 Runtime 工作狀態。匯入模組不會擷取資料、寫報告或自行監聽。

在本隔離工作樹根目錄執行：

```powershell
# 重播保存的真實快照，離線產生新的驗收報告
node work-acceptance/acceptance.mjs

# 41 項實質測試；不連網、不依賴工作當下是否仍執行中
node --test work-acceptance/acceptance.test.mjs

# 重新唯讀取得固定兩個端點、複製指定安裝來源並驗收
node work-acceptance/acceptance.mjs --capture

# 指定本目錄內另一份已保存快照
node work-acceptance/acceptance.mjs --snapshot work-acceptance/snapshots/2026-09-06T05-06-31.340Z-02f83786
```

程式以結束碼 0 表示本項資料／來源查核通過，1 表示擷取或核對失敗。報告的 `passed` 僅適用該報告的範圍，不是 Runtime 正式完成紀錄，也不是四核心整體驗收結論。新報告寫入 `runs/`，新快照寫入 `snapshots/` 的唯一目錄；既有快照不被覆寫。擷取固定使用 GET，不跟隨重新導向、不自動重試、不傳送帳密、不新增工作或寫入 Runtime。

## 固定驗收身份

| 欄位 | 值 |
|---|---|
| project_id | `78cc242a-fd7c-4d77-a895-1e5ade124618` |
| task_ref | `b3a5d42c96a94048df6084e150ecc6ed1e354f5eed1b0d1449d6e99942a1335d` |
| parent_ref | `29ab1e48bf3ac5dd3ec1ed09dbafc224d67eb45551bc8e64520cbdf6d1dadd9c` |
| 已安裝來源版本 | `4cf44233b423f0234006e15b9b6ee460b3cf2ba9` |

端點固定為 `http://127.0.0.1:4317/api/work-view?projectId=...` 與 `http://127.0.0.1:4317/api/formal-work/{task_ref}?projectId=...`，完整 URL 保存在每份 `manifest.json`。工作身份由回應的 detail、task、Ledger、metadata 與列表實際核對；未將舊版 `PLANS.md` 的 TASK-005 視為本工作的正式身份，也未改寫該文件或任何正式工作紀錄。

## 本次實測結果

- 三組真實 GET 快照均成功保存；每份原始回應保留位元組、HTTP 狀態、發出／收到的 UTC 時間、長度及 SHA-256。第二組是 [固定測試快照](snapshots/2026-09-06T04-56-12.274Z-2264f5b8/manifest.json)，第一組作為較早的更新觀測，不需要線上服務即可測試。
- [最新擷取與驗收報告](runs/2026-09-06T05-06-44.564Z-687efda3.json) 通過本項資料及來源查核。共 12 個正式節點；本工作與指定父工作存在，child.parent_id／父 children／正式 metadata.parent_ref 相符，全部可見父子連結雙向一致且無循環。
- 回應明示 `POSTGRESQL_CONTROL_PRODUCT / POSTGRESQL_TASK_LEDGER`；安裝來源確認經 `lattice_control_snapshot` 讀取。這是 API 及來源路徑證據，沒有直接 SQL 連線或 PostgreSQL 引擎健康證據。
- 最新觀測為 Ledger `SUBMITTED`／task_state `DRAFT`，Control claim 投影為 `running`、列表顯示「進行中」，`completion_verified=false`、`result_digest=null`。這些層次依安裝來源有不同用途；不能把 AI 回覆當正式完成。
- 第一次詳情收到時間為 `2026-09-06T04:54:40.792Z`，第二次為 `2026-09-06T04:56:28.949Z`。兩次身份保持不變，進度、updated_at、snapshot revision 有更新，狀態均為 `running`。已觀測到進度更新，沒有觀測到正式狀態轉換。
- [離線重播報告](runs/2026-09-06T05-03-20.295Z-a27e8949.json) 同樣通過。41 項驗收測試全部通過，包含 9 種合法狀態變體、AI 回覆完成仍待驗收、錯誤專案、缺少父工作、不一致／重複／循環父子關係、偽稱完成、來源弱化、回應竄改及輸入不可變性。

測試中的 `verified`、完成後封存、合成 result_digest 與 AI 完成文字，都是從固定真實快照複製後在記憶體建立的測試變體。它們從未寫回 HTTP 端點、快照或 Runtime，不能作為本工作已正式完成的證據。

## 已安裝來源查核

來源根目錄：

`C:/Users/f7212/AppData/Local/Programs/LATTICE/versions/4cf44233b423f0234006e15b9b6ee460b3cf2ba9/app/control-runtime/apps/lattice-control`

指定兩份檔案的 SHA-256：

| 檔案 | SHA-256 |
|---|---|
| `src/formal-work-store.mjs` | `366c9604f0547180a54f324713f96aca40b880859a94d59a6a50a2c3f56a6638` |
| `public/work-view.mjs` | `5c32baeae27374ab71601d4745cd0abd54457f176efee51554b82bb68d9b6ce1` |

完整來源副本以 `.source.txt` 保存；另保留相鄰 `store.mjs`、`execution-recovery.mjs`、`index.html` 以核對關係投影、熔斷文字與更新入口。各檔案的原始路徑、版本、時間及雜湊都在 manifest。程式只在無檔案／網路／DOM 能力的 VM 執行抽取的純 `taskProjection` 與 `workAppearance` 函式，沒有 import 產品 store 或 Runtime client。

| 來源位置 | 實際規則／結果 |
|---|---|
| formal-work-store:7、129–137 | 正式 PostgreSQL 來源宣告、Runtime snapshot 讀取及專案拒絕規則 |
| formal-work-store:26、33、39、47 | `COMPLETED && Boolean(result_digest)` 才正式完成；`TURN_COMPLETED` 僅 `codex_done` |
| formal-work-store:43、74–75 | id 取 task_ref；父工作取 metadata.parent_ref |
| store:2238–2241、2258–2264、2274–2279 | 拒絕孤立父節點、建立雙向父子關係、拒絕循環 |
| work-view:15–23 | verified 顯示已完成；codex_done 顯示等待驗收；archive 本身不等於完成 |
| formal-work-store:90、119–158 | 專案快取 15 秒；detail 直接查 Runtime，兩個 GET 非原子讀取 |
| work-view:462–467、index:839 | 以專案／revision／digest／訊息變化重繪；入口來源設定每 2 秒輪詢 |

**已發現來源限制：** `store.mjs:2315–2333` 的 tree/graph 輸出省略 `completion_verified`，但 `work-view.mjs:15,35,292` 用它識別完成後封存。因此離線完成後封存變體在列表只顯示「已封存」，詳情 Ledger 才能區分正式完成。已保留 `ARCHIVED_COMPLETION_FIELD_OMITTED` 查核結果，未修改產品，也未把封存顯示當通過的完整流程。

跨端點若狀態不符，驗收程式會回報失敗及快取／非原子讀取限制，不自動重試或編造一致資料。來源字串查核與 VM 純函式測試只證明指定版本規則，不證明瀏覽器輪詢已實際執行。

## 驗證證據及未完成範圍

- RED（先失敗）：[red-tests.log](red-tests.log)，驗收模組尚未存在。
- GREEN（修正後通過）：[green-tests.log](green-tests.log)，41／41。之後完整專案測試亦包含目前這 41 項並全部通過。
- 最新完整專案檢查：[full-verify-short-path.log](full-verify-short-path.log)、[執行結果](full-verify-short-path-result.json)。`npm.cmd run verify` 的靜態檢查通過；測試 76／79，3 項既有 Git 整合測試失敗，退出碼 1，**不能宣稱完整專案驗證通過**。
- 失敗出自 Windows Git 深路徑：`fatal: '$GIT_DIR' too big`、`Filename too long`。較長暫存路徑原先有 4 項失敗，已保留 [原始完整測試結果](full-verify-result.json) 及 [日誌](full-verify-2.log)。在同一隔離目錄內縮短暫存路徑後仍有 3 項，因此停止擴大測試；未改 Git 設定、安全控制或既有測試。此限制須在另有授權的短路徑環境驗證，與本項唯讀驗收結果分開。
- TEMP、TMP、npm cache 均導向此工作目錄內。前後 SHA-256 比對確認 55 個既有追蹤檔案皆未改動；僅新增 `work-acceptance/`。失敗證據與暫存 cache 保留，不刪除原檔。
- MCP `lattice_runtime_status` 回傳 `user rejected MCP tool call`，未重試或改用其他工具重做該健康查詢。本次走明確指定的唯讀工作資料端點，不宣稱 MCP 健康查核成功。
- 未直接 SQL 查詢、驗證 Ledger 全鏈、讀取／重新執行 result_digest 指向的正式成果；未操作瀏覽器或桌面 App、實測輪詢延遲、強制執行完成／封存／重開轉換。未做 Runtime 寫入、正式結案、push、merge、發布、部署或四核心整體完成宣告。

本工作的需求是驗收既有功能，主要成果是命令列驗收程式，因此沒有新增介面或 `startServer`。
