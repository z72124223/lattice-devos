# 專案清除流程

這是所有 LATTICE 專案共用的標準清除流程。未來刪除專案一律走同一入口：盤點 → 確認範圍 → 清除 → 必要時續作 → 逐項驗證。Codex 負責核對 ID、摘要、維護狀態及證據；使用者確認實際清除範圍，不需要自行解讀技術摘要。

工具涵蓋主 PostgreSQL、Control SQLite、登記檔案、Control／Runtime Graphify 與已綁定的独立 Bot lifecycle DB，最後可清除本次維護檔案。歸屬未知、其他專案引用或執行中工作仍明確阻擋；維護紀錄、Graphify 與 Bot 都屬 LATTICE。封存或從名單移除不得宣稱專案清空。一般 Control API 沒有遠端刪除入口。原始碼或測試通過不代表已安裝到使用者的 Runtime。

## 先確認範圍

1. 以 PostgreSQL Registry 的專案 ID 與 canonical path 核對 Control SQLite；名稱相同不代表同一專案。
2. 盤點任務、執行中工作、租約、其他專案引用、登記路徑、worktree、junction、共用 Git 目錄，以及備份與外部資源。
3. Codex 對話／附件、排程、遠端 repository／發布物、備份、獨立 Bot lifecycle DB 和外部 Graphify cache 另行逐項處理並讀回。封存對話不能填成永久刪除；沒有可用刪除介面時維持未完成。
4. 使用者確認的清除範圍必須包含永久刪除。若某次執行被工具政策拒絕，停止該操作並保留錯誤；不得改用本入口或其他工具重試以繞過拒絕。

## 已歸檔但遺漏回合紀錄

`reconcile-claim` 處理「只有第一筆 THREAD_BOUND、原始回合實際已完成並歸檔」的缺漏。它不重新註冊專案、不恢復派工資格、不啟動 Codex，也不把工作標為正式驗收完成。一般 OBSERVE 的新派工檢查保持不變。

輸入檔為 `{ "nativeBinary": "<目前安裝的絕對路徑>", "request": {...} }`；request 必須含 `schema: "lattice.project-purge.request.v1"`、`action: "reconcile-archived-claim"`、`authorization: "RECONCILE_ARCHIVED_CLAIM"`、精確 `projectId/taskRef/claimId/operationId`、`expectedSequence: 1`，以及 `parentArchive/threadArchive` 的絕對路徑和 `parentSha256/threadSha256`。`--confirm` 核對整份輸入檔的 SHA-256；使用目前受支援的維護連線及明確 `CODEX_HOME`。先讀取原生對話確認無執行中回合，再以 `project:purge reconcile-claim --input archive-proof.json --confirm INPUT_SHA256 --maintenance-offline` 執行。

目前只支援 Windows：原生程式要求兩份檔案位於 CODEX_HOME 的 `archived_sessions`，拒絕 reparse point，以不允許寫入或重新命名的檔案分享模式持續鎖定至交易結束。程式逐項核對原主控的 create_thread、成功 THREAD_BOUND、原生進行中與完成回執，以及歸檔中的原始回合與所有後續回合終止事實；子代理尚無終止回報也拒絕。這是歸檔事實驗證，不是所有背景程序或其他主機的存活證明；專案維護仍須停止其寫入者。

PostgreSQL 必須已 STOPPED。維護入口鎖住 admission，核對既有 Registry／task／claim 與原始綁定摘要，沿用原有 SQL 契約，在同一交易補登 DISPATCH_STARTED、TURN_BOUND、TURN_COMPLETED、ARCHIVED，全部明標為事後對帳。任何不同歷史、並行修改或 SQL 失敗均整筆回滾；同一操作與證據可精確重播。它不更動 SQL schema、purge guard、任務 ledger 或 SQLite 名單。完成後重新盤點；對帳成功不解除工具政策對實際刪除的限制。

## 支援範圍與拒絕條件

- PostgreSQL 保留資料驗證在同一交易內以唯讀 cursor 每批 4,096 列串流；全量列數與總 JSON 不再受舊版 100,000 列／64 MiB 上限限制。資料表使用舊 `BTreeMap` 字節順序，列仍使用 PostgreSQL `COLLATE "C"` 排序；scope v2/v3/v4、afterDigest 及保留資料摘要維持原序列化，舊計畫與收據可雙向核對。每列仍最多 64 MiB、每次完整快照最多 60 秒，單條 SQL 取原期限與剩餘快照期限的較小值（一般清除 30 秒、圖譜盤點 15 秒），零或負剩餘時間直接拒絕，不能變成無期限。超時不回傳部分摘要；整體 CLI 期限仍在。舊 Registry 尾段參照盤點與歸屬索引另保留原有容量限制，不代表任何大小、任何資料種類皆可清除。
- 2026-10-10 初次在含 584,134 筆 Graphify records 的既有資料庫上完成唯讀預覽，耗時約 64 秒。後續核對發現，當時 4 筆 decision 引用其實屬於目標專案，是 selector 漏表造成誤判；9 份舊 analysis 的兩組配置也已由實際 Git hash、工作目錄與執行環境路徑精確重算吻合。這些修正不代表已刪除正式專案。
- PostgreSQL：在既有 Store `STOPPED` 維護狀態，以專用 migrator 連線執行。舊版 `VERIFIED_SUFFIX_V1` 只接受目標命令構成全域歷史最後一段，清除後回到原始保留前綴，完整重播仍從起點開始。沒有選擇資料保留政策的舊計畫維持此限制。
- 選擇 `registryPolicy: "MINIMAL_ATTESTATION"` 後，`ATTESTED_EPOCH_V1` 可處理交錯歷史。清除前完整驗證舊歷史／前次受信任基準與新命令；清除後一般保留專案的原始命令、語意收據及 PostgreSQL 持久化收據保持原值，存於新的歷史基準。必須移除的跨專案歷史命令，在預覽列出命令承諾與 record-set 摘要，授權綁定同一範圍；舊指令 ID 以摘要保留並拒絕任何內容的重送。新的指令 ID 可登記已釋放的身分。
- 這項政策的歷史保證為 `ATTESTED_FROM_SEAL`，新的命令從已驗證基準完整重播；不是刪除前全域歷史仍能從零重播。只有一份當前基準；下次清除會再次過濾，不能保留可能含新刪除目標的舊基準原文。其他專案的**目前狀態**仍引用目標時維持 `REGISTRY_CURRENT_SURVIVOR_REFERENCE`，必須先走該專案正常調和程序，不可用歷史刪除授權改掉它的現況。
- 新基準由資料庫外的主機憑證固定其摘要與 epoch，資料庫不能自我宣告受信任。預設 Windows 路徑是 `%LOCALAPPDATA%/LATTICE/registry-epochs/<database-identity>/registry-epoch.anchor.json`；主機設定 `LATTICE_REGISTRY_ANCHOR_ROOT` 可改共同根目錄，所有 reader 與維護工具必須一致，不能由 DB、計畫或刪除要求自行指定。憑證不得位於刪除根目錄，路徑別名與硬連結均拒絕。Unix 主機預設使用 XDG_STATE_HOME 或 HOME/.local/state，未宣稱已做 Unix 整合驗收。
- 摘要不是匿名化，低熵 ID 可能被猜測。新 attested 維護收據只存操作 ID 的承諾摘要；回覆中的原 ID 只供同一呼叫者續作。旧維護收據不自動改寫，報告列出待檢閱筆數。主機憑證僅防資料庫單邊跨 epoch 回滾，不防同一 OS 使用者同時改檔案和 DB，也不防同 epoch 新命令尾端的回滾。
- task streams／ingress／Control product 等已實作的固定資料閉包一起刪除。未知表、尚未支援的資料種類及跨範圍引用會列入 blockers；不可把 blocker 當成已清除。預覽的 counts 是實際範圍，並非所有未來擴充功能的涵蓋承諾。
- SQLite：只接受既有精確 schema profile；工作、事件、內部關係、登記、觀察及其附表在交易中清除。名單已先被移除時，必須由當次 PostgreSQL 預覽提供相同 ID、路徑與 scope digest，才能接手殘留資料；名單不存在本身不能充當清除成功。其他專案的原始 rows 與收據內容必須不變（共享 decision_state 的明示轉換見下）；同一路徑的其他專案登記仍會阻擋刪除。
- 目標有 installation receipt 或可證歸屬的 decision 時，Windows 自動採 `REBUILD_SURVIVORS_V1`：不修改或停用原資料庫的 append-only trigger，以精確原 schema 重建保留資料。逐項核對原 row、rowid、事件序號高水位、完整性、FK、索引、trigger 與檔頭設定，再同目錄替換舊檔。重建前以系統 Windows PowerShell 唯讀檢查 owner/group/DACL，必須與新暫存檔完全相同；自訂 ACL、無法讀取或中途變動都拒絕，不能為通過清除而放寬權限。SACL 稽核設定未驗證；此版未提供權限複製或非 Windows 的重建轉接器。一般開啟會恢復 WAL 模式；SQLite 核心 schema 為 v7，新增決策歸屬 extension 需要相容的新讀寫端。
- Control SQLite 新決策必須明確帶 `owner: {kind: "PROJECT", projectId: "精確 Control 專案 ID"}` 或 `{kind: "GLOBAL"}`。首次寫入在同一交易內安裝固定 sidecar extension，綁定 decision ID 與專案 FK；同 scope 的所有 subject／lineage 必須是同一 owner。不要將專案資料標成 GLOBAL 來規避歸屬。原 decisions 欄位、列摘要與 immutable triggers 不變；新請求摘要包含 owner。舊版已開啟的 writer 會被新增 INSERT guard 拒絕，舊版 constructor 會拒絕擴充後的 exact profile。使用前應更新全部相關讀寫端；首次寫入失敗會將 extension 和 sidecar 一起回滾。
- 舊列不會自動認領；既有無 owner 的 exact request 仍可重播，但不能在其 scope 新增列。任何缺 owner／混合 owner 的 scope 都阻擋清除，預覽列出未知 scope 的摘要與筆數，不以 scope 字串猜測。只移除完整且專屬於目標的 scope；其他專案／GLOBAL 引用目標 decision ID、專案 ID 或路徑仍阻擋。正常 PostgreSQL 決策原本就有結構性 project scope；若提供 owner，必須與其專案一致。
- SQLite 重建保留其他決策及 sidecar 的原始 row、rowid、request digest；共享 `decision_state` 依原演算法重算，revision 是**存活列數**，清除後可下降。保留不含專案內容的 before/after revision、digest、operation digest 與驗證時間。舊全域 read/search packet 因身分不符而拒絕，必須重新讀取；B 的 exact mutation replay 回原決策和新的全域 revision/digest，沒有重新證明刪除前的全域封包。
- 移除決策的 client_request_id 僅保留 domain SHA-256 tombstone，且檢查先於 exact replay。相同請求 ID 即使換 owner 或內容也拒絕；新 ID 配有效 owner 與新 state 可寫入新決策，這不是對相同內容的永久禁令。此行為列在 preview 的 `decisionRetirement`。未驗證的 legacy 資料仍標未知，不增加 SQLite epoch 來掩蓋歸屬問題。
- 檔案：只接受權威清單中的絕對路徑；拒絕使用者家目錄、磁碟根、工具自身、其他專案、重疊根、祖先 junction、未支援的巢狀 repository 等。junction／symlink 僅移除連結，不追蹤目標。檔案預覽有數量、深度、manifest 大小上限。
- 硬連結：目前在盤點及執行前驗證時拒絕 `nlink > 1` 的檔案或連結，即使所有名稱看似位於同一專案。移除其中一個名稱會改變共用檔案的連結數與時間戳；本版沒有完整的硬連結歸屬與續作轉接器，因此須在任何 PostgreSQL／檔案刪除前阻擋，不能先刪一半再卡住，也不能略過時間戳驗證或改動外部連結以強行通過。
- Control 圖譜磁碟快取：從共用快取目錄盤點全部直接子目錄，以 graph.json 格式、project_id、source_root、目錄鍵及內容摘要建立歸屬，涵蓋同專案不同 checkout。只有確定屬於目標的快取才加入相同檔案清單；內容變更、新增目標快取、未知目錄、缺失標頭、連結及超出盤點上限均阻擋。共用快取目錄與專案刪除根重疊也阻擋，以保護其他專案快取。此項不涵蓋 Runtime 的共用 Graphify 記憶體／索引或仍運作的 Control 記憶體快取，執行仍需停止相關寫入者。
- 這不是磁碟安全抹除：SQLite／PostgreSQL 的備份、WAL、儲存媒體殘留及外部副本不由本入口保證消失。它驗證的是支援範圍的邏輯資料與路徑不再存在。

Runtime Graphify 的檔案範圍可由 `runtimeGraphWorkDirectory` 或既有 `LATTICE_GRAPHIFY_WORK_ROOT` 指定。原生程式以 Registry 路徑重用 Runtime 的 canonicalize 與相同 domain hash，將 `sources/<source key>` 加入固定刪除清單；其他 source 目錄保留。只在此已綁定範圍允許空的巢狀 `.git` 快照邊界，含任何 Git metadata 的目錄仍拒絕。未知舊平鋪格式、別名、無法核對的來源均阻擋。未配置 work root 不代表没有快取。主 Store 的實際 Memory 表另行唯讀盤點；有舊 analysis 時仍須證明其歸屬，不能把固定的 `task032-delivery` 當成 Registry ID。

Bot lifecycle 使用**獨立 PostgreSQL cluster**，安裝器禁止與 Store 共用服務。協調層讀取既有 `%USERPROFILE%/AppData/Local/LATTICE/bot-lifecycle-postgres/v1/identity.json` 的 port、runId、systemIdentifier，或接受同形的 `botService` 設定（不含密碼）。原生端核對 system identifier、專用 schema、原函式及權限；未配置或讀取失敗保持未知。明確執行原生 `install-bot-ownership`、授權值 `INSTALL_BOT_PROJECT_OWNERSHIP` 才加入 ownership extension。此後新的 `lattice-runtime bot-lifecycle` register 以既有主 Store 環境設定驗證 Registry，持有讀鎖直到 Bot 登錄與 binding 一起提交。這是應用程式的跨資料庫驗證，不是假稱跨叢集 FK。舊自由 project key 不會自動認領。

Bot 預覽列出待刪角色的 owner、revision／generation 與永久封存舊 project/role 組合的決策。Codex 用原生 `read_thread` 取得精確 owner 的最新 idle/completed、pending=0、in-flight=0 證據；`botBoundaryPath` 必須在原盤點設定中列出，apply/resume 的 `--bot-boundaries` 使用該路徑。證據有效期五分鐘，不能把測試 envelope 當作正式證據。角色必須沒有未完成交接或執行步驟，主 Store 必須 STOPPED。Bot 同交易寫最小 hash 防重憑證、清除目標 roles/events/bindings、驗證保留 rows 完全相同；先完成 Bot，再清除 Registry。舊 register/finish 寫入受到資料庫保護，防止重新建立已退役組合。這不會封存對話或停止程序。提交回覆遺失時先核對同 scope 的 receipt；Bot 已清而主 Store 尚未完成仍是部分完成。

## 執行入口

PostgreSQL 決策依 `project_id` 清除完整 lineage，保留其他專案的原始 row 與 sequence。存在存活列的 FK 或文字引用時仍整筆阻擋。須先在離線維護狀態執行原生 `install-decisions`，授權值 `INSTALL_DECISION_PURGE`，安裝精確的後繼 catalog；只支援 `MINIMAL_ATTESTATION`。清除維持全域 revision 的寫入高水位，以既有算法重算存活決策 digest，並保留 decision/request ID 的分域 SHA-256 防重用紀錄。舊 expected pair 失效，下一次正常寫入從原高水位繼續；摘要並非匿名化。未安裝時仍能預覽歸屬與數量，但不能執行清除。

Graphify 的 PostgreSQL 歸屬判定支援已驗證的同儲存庫工作目錄。每次預覽與執行都重新核對實際 canonical Git common directory，若任何存活 Registry 專案使用同一儲存庫，整組阻擋。Windows junction 僅供唯讀身分核對；不會因此放寬檔案刪除的連結邊界。身分承諾值綁入 scope digest，路徑改指向會使舊計畫失效。

可由受控的本機環境設定 `LATTICE_GRAPHIFY_PURGE_SOURCE_HISTORY` 提供至多 16 組歷史輸入，JSON 物件只允許 `sourceRoot`、`runtimeRoot`、`gitExecutable` 三個絕對路徑（總長上限 16 KiB）。程式重新讀取 Git 實體 hash、來源身分與 Runtime 配置算法，且核對分析 commit 確實存在；只選取重算摘要完全相同的分析。不接受直接指定「某個 hash 屬於目標」。缺失輸入、未知配置與真正跨專案引用仍阻擋。這是主 Store 資料的歸屬證明，不能拿來宣稱無標頭的舊磁碟快取也已確定歸屬。

原生唯讀預覽在同一次 PostgreSQL snapshot 中產生主庫與 Graphify 摘要，避免再排序讀取所有 Graphify records。仍維持既有摘要位元組、單筆上限及 60 秒 snapshot 時限。

安裝維護 schema 前，必須先確認所有 Store 讀取者都使用包含本版精確維護 catalog profile 的相容版本。舊版會拒絕新增的維護表；只換清除 binary 而保留舊讀取者，無法視為完成部署。此工具不會自行更新或重啟既有服務。

先編譯維護 binary：

```powershell
cargo build -p lattice-runtime --bin lattice-project-purge
```

原生 binary 從標準輸入接收 JSON，連線只使用既有 `LATTICE_TASK019_PORT`、`LATTICE_TASK019_RUN_ID`、`LATTICE_TASK019_PASSWORD`。不得把密碼放進命令列、計畫、日誌或 repository。新資料庫須由有權限的維護人員執行其 `install` action，建立獨立清除 receipt schema；不會自動關閉 Store 或擴張 runtime 權限。

Node 協調入口使用目前專案要求的 Node 版本。準備 config JSON，所有路徑必須絕對；計畫與進度檔放在清除範圍外：

```json
{
  "projectId": "<Registry 專案 UUID>",
  "registryPolicy": "MINIMAL_ATTESTATION",
  "nativeBinary": "C:/maintenance/lattice-project-purge.exe",
  "databasePath": "C:/maintenance/control.db",
  "statePath": "C:/maintenance/purge-progress.json",
  "codeGraphCacheDirectory": "C:/Users/example/AppData/Local/LATTICE/control/code-graphs",
  "protectedRoots": ["C:/another-project"],
  "externalResources": [
    {"kind": "codex", "reference": "<對話 ID>", "source": "Codex 對話清單讀回"}
  ]
}
```

```powershell
npm.cmd run project:purge -- inventory --input config.json --plan purge-plan.json
npm.cmd run project:purge -- apply --plan purge-plan.json --confirm <預覽的 digest> --maintenance-offline
npm.cmd run project:purge -- resume --plan purge-plan.json --confirm <同一個 digest> --maintenance-offline
npm.cmd run project:purge -- status --plan purge-plan.json
npm.cmd run project:purge -- verify --plan purge-plan.json
```

`preview` 與 `inventory` 是同一唯讀盤點動作，只保存可檢閱計畫；已有計畫檔不會被覆寫。PostgreSQL 盤點以唯讀交易取得一致快照，可在服務運作中或尚未安裝維護 schema 時執行，仍回傳實際 counts；分別以 `MAINTENANCE_OFFLINE_REQUIRED`、`MAINTENANCE_EXTENSION_REQUIRED` 阻擋清除。已安裝但不符合精確 catalog／identity 的維護 schema 仍會拒絕，不能當成未安裝。盤點不會安裝元件、停止服務或修改資料庫。Serializable 唯讀交易使用 `DEFERRABLE` 等待安全快照，保留既有 30 秒查詢上限；逾時或查詢失敗仍回錯誤，不產生可執行計畫。

停止服務並安裝相容的維護 schema 後，必須另存新盤點，重新確認其範圍。scope digest v2 納入維護元件是否存在及 admission 狀態，不能沿用改變前的摘要；尚未執行的舊版 v1 計畫也須重建。已提交的舊 operation 仍按原 project／digest 綁定及 afterDigest 驗證續作，不會重新執行 PostgreSQL 刪除。

`control:project delete` 和 `control:project purge` 轉交到同一入口，例如 `npm.cmd run control:project -- delete inventory --input config.json --plan purge-plan.json`。沒有略過盤點的快速刪除選項。

`BLOCKED` 必須先解決列出的原因並重新預覽。`apply` 必須使用完全相同的 digest，並確認 Control／專案寫入者已停止、檔案範圍保持靜止。Node 的路徑 API 不能對抗惡意並行 rename；這不是可在線上任意執行的安全保證。`resume` 沿用相同計畫、摘要及維護要求，不會自動重新盤點或解除鎖。

`externalResources` 可省略，僅接受 `kind`、`reference`、`source`。七種分類為 `codex`、`automations`、`git`、`graphify`、`botLifecycle`、`backups`、`maintenance`。這個歷史欄位名稱為相容而保留，不代表七類皆是外部：報告的 `ownership` 區分 LATTICE、Codex 及混合範圍，LATTICE 未驗證項標成 `VERIFICATION_REQUIRED`，並列出具體下一步。沒有填資源不等於不存在。呼叫者不能自行填入已刪除、已驗證或完成狀態來取得通過；`latticeScopeComplete` 與全域 `complete` 分別保留，前者須經 finalize 收尾才可達成，後者包含外部範圍。

`codeGraphCacheDirectory` 可省略，預設與 Control 相同：`%LOCALAPPDATA%/LATTICE/control/code-graphs`。若 Control 使用自訂目錄，必須提供實際目錄。報告將這個可驗證磁碟範圍獨立列為 `controlCodeGraph`；即使通過，外部 `graphify` 分類仍未完整驗證。沒有此盤點欄位的舊計畫保持原範圍，不能據此聲稱已清掉快取。

使用最小驗證憑證前，以原生入口的 `install-epoch` action 與 `INSTALL_REGISTRY_EPOCH_MAINTENANCE` 授權安裝精確的可選 catalog；仍須已停止 Store。它包含一般清除 receipt schema，不改 admission 或角色權限。舊 Runtime 不支援此新增 catalog，須先準備相容讀取者；schema 安裝不是 Runtime 部署。安裝完成後重新產生預覽，核對 `history` 的 redactedSurvivorCommands、assurance 與限制，再確認同一計畫。

`apply`／`resume`／`status` 在本機階段完成時 exit 0，報告維持 `PARTIAL`、`complete: false`；全域 `verify` 對此回傳 exit 2。阻擋或錯誤為 exit 1。所有已配置、可證歸屬的 LATTICE 邏輯資料讀回通過後，執行 `finalize --plan plan.json --confirm DIGEST --maintenance-offline` 清除原盤點綁定的 config、plan、progress 與 native boundary 檔。未知 legacy decisions、未驗證 Bot／Graph 或未完成清除都阻擋收尾。

收尾產生 `LOGICAL_SCOPE_COMPLETE`／`latticeScopeComplete: true` 的最小憑證；全域 `complete` 仍為 false。這個契約指已配置的運作中 LATTICE 邏輯儲存，不涵蓋舊備份、WAL、物理媒體、其他主機副本及 Codex 對話永久清除。已批准的 Registry seal、操作收據、防重 tombstone 與收尾憑證是有意保留的驗證產物，不是待刪專案內容；低熵識別的雜湊仍可被離線猜測。

finalize 的檔案允許清單在原 preview 綁定，收尾不能追加任意路徑。它核對 config 原始雜湊、計畫與進度歸屬，再固定各檔案內容及身分，逐檔記錄 intent、移除、讀回。原路徑內容或身分變更就保留並阻擋。最後先同步最小憑證暫存檔，再原子取代含內容的恢復 manifest；憑證保留前一 manifest 摘要。`resume-finalize --finalization <盤點列出的路徑> --confirm DIGEST --maintenance-offline` 可在 plan／progress 已被移除後續作；只接受精確 predecessor 相連的暫存紀錄。兩份都不存在不算成功，未知鎖不自動解除。`verify-finalization --finalization <路徑>` 驗證已保留憑證，並非持續重查已被移除計畫的專案。此機制不防同使用者篡改，也不保證突然斷電的自動恢復。

## 交易、部分失敗與續跑

1. 以獨占 operation lock 防止同一進度檔同時執行；一般清除在 SQLite `BEGIN IMMEDIATE` 內核對原始摘要並保持鎖。收據重建則先建立資料庫旁的 `.purge-swap` 維護標記、完成 WAL checkpoint 並切換 DELETE journal，再持有 `BEGIN EXCLUSIVE`；新 Control 在開啟資料庫前見到維護標記會拒絕。
2. 先核對檔案 manifest、PostgreSQL scope digest／blockers，再清除已綁定 Bot，最後執行主 PostgreSQL 清除交易。
3. 以 operation ID 與 scope digest 讀回 PostgreSQL receipt，接著逐項移除檔案；最後才提交 SQLite 清除。
4. 三個儲存系統無法組成單一原子交易。中斷、鎖檔、權限錯誤或回覆遺失都記為 `INCOMPLETE`；不能宣稱已回復原狀。已提交的 PostgreSQL 清除不能由 SQLite rollback 撤銷。
5. 用原計畫、原 operation ID 與進度檔續跑。PostgreSQL 回覆遺失時先讀回 receipt，不再次刪除。每個檔案移除前先寫入 intent 並同步進度，移除後再保存結果；若在兩者之間崩潰，續作只會對原計畫中那一項的缺失進行核對。未知路徑、內容變更及預覽以外的變動仍會拒絕。歷史 receipt 還要符合目前資料的 `afterDigest`；即使是保留專案後續合法變更，也須重新核對，不能只憑舊成功紀錄宣稱目前已清空。
6. 若程序異常留下 `.lock`，先確認該 operation 已無執行程序並讀回所有階段，再處理鎖檔；程式不會自動猜測 stale lock。不得刪除未知鎖或啟動第二個 writer。
7. 三階段讀回通過只回報 `SCOPED_PURGED`。`externalCleanup: NOT_VERIFIED` 明確保留外部清理待辦；只有外部逐項驗證也完成，才可向使用者說「整個專案已清空」。

Attested Registry 在 PG 刪除前先寫外部 `Pending(previous, next, operationDigest)`，再用單一 PG 交易寫新基準、ID 防重表與清除收據。PG 提交後，必須比對同一新基準及已提交收據，才將主機憑證改為 Active。正常 Runtime 遇 Pending 一律拒絕。提交前中斷只能沿用同操作／摘要驗證 previous；提交後中斷只能以匹配的收據完成 next，不可用「舊狀態驗證失敗」猜測已提交。缺少主機憑證、兩邊不符或未知鎖都維持阻擋，不從 DB 自動重建。人工處理時先停住所有 reader/writer、保存原錯誤、核對原計畫與兩邊實際狀態；沒有独立可信證據時不能補造新憑證或重新開始刪除。

這裡的中斷續作指程序崩潰或一般 I/O 失敗；不保證突然斷電後可自動續作。進度檔雖先做檔案同步再原子替換，但父目錄項與目標檔案刪除的斷電落盤順序尚未驗證，尤其 Windows 不能由目前 Node API 假定相同保證。重啟後若進度與檔案不符，維持阻擋並人工核對，不補造已移除紀錄。

SQLite 重建在 PG 與檔案成功後才填入 `.purge-next` 保留資料檔；關閉來源與暫存資料庫後，兩側的 WAL／SHM／journal 必須不存在，才保存綁定原計畫的新舊檔摘要並替換。換檔前失敗可從原檔與暫存檔繼續；換檔後但移除標記前失敗，只有新檔摘要正確且暫存檔已消失才接受完成。未知檔案、缺失 DB、錯誤計畫、旁檔、連結或內容變動一律阻擋，不補造新資料庫。未完整落盤的 marker 暫存檔、尚未登記身分的 staging、或中斷重建留下的 journal 需要先依原計畫人工核對，不能任意刪除來解鎖。成功時不保留原 DB 副本；OS 區塊、備份及稽核設定仍不在抹除保證中。舊客戶端不識別此標記，維護前必須停止全部寫入者；沒有宣稱已排除所有 OS handle。

Graph PostgreSQL 歸屬使用 Runtime 原本的來源設定摘要：同一個路徑正規化函式、Git 執行檔內容摘要及平台設定。維護 binary 自行重算，請求不能提供自選摘要。它會檢查其他 Registry 專案沒有共用來源，再按來源清除 analysis、records、retrieval audits、receipts 與 reflections；保留資料逐列摘要必須完全不變。共用舊 Graph project ID 不再等於共用所有權。無法重算的歷史設定、其他專案跨向待刪收據的引用、未分類 gateway command 都不能當成無關資料；其中 ownership 或引用阻擋仍在時不會刪除。這項驗證沒有宣稱使用者任意刪去來源後仍能反推所有權。

計畫／進度檔本身保留路徑及清除證據；它們也是最終資料保留決策的一部分。不要將實際專案計畫、資料庫或含機密的測試輸出提交 Git。

## 套件交付

`scripts/lattice-bundle.py build` 可用三個成組參數加入清除功能：`--project-purge-binary`、`--project-purge-sha256`、`--project-purge-source`。最後一項是提供本版 Node 依賴檔案的 repository 根目錄。三者必須同時提供；打包及驗證會核對固定的 binary／CLI 檔案清單與 SHA-256，並將 `project_purge` 能力綁到同一套件的 Runtime hash。套件內直接使用 `node apps/lattice-control/src/project-purge-client.mjs --help` 查看入口。

此能力仍須明確加入套件；舊套件維持相容，不會因 repository 新增檔案而自動取得刪除功能。套件驗證也不會解除資料庫、共用 Git、交錯歷史或工具政策的阻擋。

啟用 Bot ownership 的候選套件同時傳入 `--lifecycle-binary` 與 `--lifecycle-sha256`，納入相容的 `bin/lattice-runtime.exe`；打包與讀回驗證會核對此 companion 的固定檔名與 SHA-256。既有主機使用舊 CLI 時，不可因維護 binary 已更新就假定正常 Bot 登錄也已具備 Registry 綁定。

只需要本機離線維護工具時，可用 `build-maintenance`，參數為 `--bundle`、`--runtime`、`--runtime-sha256`、`--node`、三個 purge 參數，以及 `--vc-redist`／`--vc-license`／`--vc-redist-list`。固定清單包含相容 Runtime、清除 binary、Node/CLI 依賴閉包、VC runtime 及授權來源；不重複複製 PostgreSQL、Python、Git 或 Graphify。用 `verify-maintenance --bundle ... --sha256 ...` 核對。這是本機候選維護包，不能交給一般 `install` 假裝完整依賴部署。

## 驗證

```powershell
node --test apps/lattice-control/test/project-purge*.test.mjs
cargo build -p lattice-postgres-store --example project_purge_fixture
cargo build -p lattice-runtime --bin latticed --bin lattice-project-purge --bin lattice-runtime
cargo build -p lattice-runtime --example project_purge_epoch_fixture
pwsh -NoProfile -File scripts/test-project-purge-postgres.ps1 -PurgeBinary <lattice-project-purge.exe 絕對路徑> -SeedBinary <project_purge_fixture.exe 絕對路徑> -RuntimeBinary <latticed.exe 絕對路徑>
```

PG harness 使用新的 loopback cluster、合成專案與任務，保留 `.lattice` 下的證據，不連正式資料庫。Windows 檔案測試包含實際鎖檔及部分失敗續跑。協調層測試使用真 SQLite／檔案與受控 native adapter；真 PG fixture 的結果須另外報告，不能把 mock 當成完整部署驗收。

`-Scenario inventory` 可單獨驗證運作中／未安裝維護元件的盤點不改動資料、離線要求仍生效，以及狀態改變後的舊摘要被拒絕。

`-Scenario bot-inventory` 使用全新且沒有 Store 的獨立 cluster，經真實安裝器和正常 Bot register API 寫入合成憑證，驗證不存在、空資料庫、有資料、文字 ID 不符，以及服務身分不符；不讀取使用者的 Bot 服務。`coordinator-absent` 另以真實 native source key 驗證 Runtime 快照清除、空 Git 邊界及另一專案快照原文保持不變。

`-Scenario epoch` 與 `-Scenario epoch-reference` 使用各自的新 cluster，驗證交錯歷史／跨專案拒絕紀錄、外部憑證、原始持久化收據精確重送、兩次清除及新命令。Pending 案例由 fixture 寫入真實預覽綁定的提交前／提交後檔案狀態，驗證後續程序恢復；這是中斷狀態模擬，不是突然斷電驗收。`project_purge_epoch_fixture` 使用 Runtime 的實際原生檔案識別及 Store，要求合成資料目錄 marker 和 fixture opt-in，且拒絕已知正式埠；不會加入交付套件。

`-Scenario survivor-reference` 以正式 Registry API 建立跨專案的 duplicate-denied 憑證，驗證盤點回傳引用筆數、清除被拒絕、資料及 receipt 不變、另一程序仍能重播原歷史。`-Scenario coordinator-absent` 另涵蓋真 PostgreSQL／SQLite／檔案及不同 checkout 的 Control 圖譜快取清除，並核對保留專案快取原文不变。

`-Scenario upgrade -LegacyPurgeBinary <舊版維護 binary 絕對路徑>` 使用真正舊版 binary 產生 v1 摘要與清除 receipt，再以新版 binary 讀回及重試原操作，驗證相容性。舊、新 binary 都會複製並核對 SHA-256；未提供舊版 binary 的一般測試不包含此驗證。

`-Scenario bot-purge` 在兩個全新 cluster 驗證正常 Bot 登錄歸屬、原生閒置證據、Bot 先於 Registry 清除、Control SQLite 決策完整 lineage 清除與保留列號、角色防復活、CLI 續跑及 maintenance finalize；`all` 包含此案例。所有情境保存 Node source hashes 並在完成時核對未變。

`-Scenario streaming -LegacyPurgeBinary <串流化前相容 binary 絕對路徑>` 比對 Unicode／大小寫／數字字串的小型原生圖譜，驗證新舊預覽完全相同、兩方向收據讀回及舊計畫接受。`-Scenario streaming-large` 使用正常 Graph API 在兩個專案寫入合計 110,000 列、超過 64 MiB；舊版應容量拒絕，新版仍完整檢查跨專案引用，刪除目標 55,000 列後核對保留 55,000 列原文摘要及原生 receipt，並重試同操作。兩種情境各使用新隔離 cluster，不連線正式庫；輸出記錄執行耗時及新舊 binary 雜湊。它們須明確選擇，不由未指定相容舊 binary 的 `all` 推定已驗證。


### 舊資料一次遷移到目前格式

維護 binary 提供 `preview-bot-adoption` 與 `adopt-bot-ownership`：共同輸入為固定 schema、`projectId` 與已核對的 `botService`；採納另須 `authorization: ADOPT_EXISTING_BOT_OWNERSHIP` 及預覽的 `expectedSnapshotDigest`。先用既有 `install-bot-ownership` 安裝固定 sidecar，主 Registry 必須維護離線。工具鎖住 Registry、Bot 原表與 sidecar，逐筆核對原始 register、request 摘要、連續 revision、收據身分與最後狀態，才新增當前歸屬。錯誤 UUID、事件缺口、摘要變動、已退休或跨 Store 綁定一律拒絕；roles/events 不變，可安全重跑。這只證明保存歷史與當前 Registry 的歸屬，不證明原生對話已停止，也不核准清除。

舊 Control 圖譜缺少 `cache_digest` 時，從已驗證的 source/commit 重新分析到暫存目錄；新摘要驗證成功且原檔未變才在維護時段替換。不為舊內容直接補摘要。舊 Runtime commit 目錄只在逐檔符合來源 Git blob、staging 對應同一 snapshot，且內容清單已保存後，以不覆寫的 rename 移入 `sources/<sourceKey>/<commit>`；中斷後核對原／目標與同一清單續跑，前後內容必須相同。這是一次資料遷移，正常讀取器不保留舊格式分支。

維護 source key 可唯讀解析 Registry junction，但需逐一排除其他 Registry 來源使用同一 key；一般 Graphify admission 仍拒絕 junction。檔案清除繼續只處理已計畫的 link 本身，不因解析歸屬而穿透刪除目標。
