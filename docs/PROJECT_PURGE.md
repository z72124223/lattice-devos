# 專案清除流程

這是所有 LATTICE 專案共用的標準清除流程。未來刪除專案一律走同一入口：盤點 → 確認範圍 → 清除 → 必要時續作 → 逐項驗證。Codex 負責核對 ID、摘要、維護狀態及證據；使用者確認實際清除範圍，不需要自行解讀技術摘要。

目前工具能清除已支援的 PostgreSQL、Control SQLite 及檔案範圍；尚未實作的 LATTICE 管轄資料與真正外部資源分別列在同一份報告。維護紀錄、Runtime Graphify 和獨立 Bot lifecycle DB 仍屬 LATTICE，不能因尚未有轉接器就稱作外部事項。封存、從名單移除及本機部分清除都不得宣稱為整個專案清空。一般 Control API 沒有新增遠端刪除入口。原始碼或測試通過也不代表已安裝到使用者的 Runtime。

## 先確認範圍

1. 以 PostgreSQL Registry 的專案 ID 與 canonical path 核對 Control SQLite；名稱相同不代表同一專案。
2. 盤點任務、執行中工作、租約、其他專案引用、登記路徑、worktree、junction、共用 Git 目錄，以及備份與外部資源。
3. Codex 對話／附件、排程、遠端 repository／發布物、備份、獨立 Bot lifecycle DB 和外部 Graphify cache 另行逐項處理並讀回。封存對話不能填成永久刪除；沒有可用刪除介面時維持未完成。
4. 使用者確認的清除範圍必須包含永久刪除。若某次執行被工具政策拒絕，停止該操作並保留錯誤；不得改用本入口或其他工具重試以繞過拒絕。

## 支援範圍與拒絕條件

- PostgreSQL：在既有 Store `STOPPED` 維護狀態，以專用 migrator 連線執行。舊版 `VERIFIED_SUFFIX_V1` 只接受目標命令構成全域歷史最後一段，清除後回到原始保留前綴，完整重播仍從起點開始。沒有選擇資料保留政策的舊計畫維持此限制。
- 選擇 `registryPolicy: "MINIMAL_ATTESTATION"` 後，`ATTESTED_EPOCH_V1` 可處理交錯歷史。清除前完整驗證舊歷史／前次受信任基準與新命令；清除後一般保留專案的原始命令、語意收據及 PostgreSQL 持久化收據保持原值，存於新的歷史基準。必須移除的跨專案歷史命令，在預覽列出命令承諾與 record-set 摘要，授權綁定同一範圍；舊指令 ID 以摘要保留並拒絕任何內容的重送。新的指令 ID 可登記已釋放的身分。
- 這項政策的歷史保證為 `ATTESTED_FROM_SEAL`，新的命令從已驗證基準完整重播；不是刪除前全域歷史仍能從零重播。只有一份當前基準；下次清除會再次過濾，不能保留可能含新刪除目標的舊基準原文。其他專案的**目前狀態**仍引用目標時維持 `REGISTRY_CURRENT_SURVIVOR_REFERENCE`，必須先走該專案正常調和程序，不可用歷史刪除授權改掉它的現況。
- 新基準由資料庫外的主機憑證固定其摘要與 epoch，資料庫不能自我宣告受信任。預設 Windows 路徑是 `%LOCALAPPDATA%/LATTICE/registry-epochs/<database-identity>/registry-epoch.anchor.json`；主機設定 `LATTICE_REGISTRY_ANCHOR_ROOT` 可改共同根目錄，所有 reader 與維護工具必須一致，不能由 DB、計畫或刪除要求自行指定。憑證不得位於刪除根目錄，路徑別名與硬連結均拒絕。Unix 主機預設使用 XDG_STATE_HOME 或 HOME/.local/state，未宣稱已做 Unix 整合驗收。
- 摘要不是匿名化，低熵 ID 可能被猜測。新 attested 維護收據只存操作 ID 的承諾摘要；回覆中的原 ID 只供同一呼叫者續作。旧維護收據不自動改寫，報告列出待檢閱筆數。主機憑證僅防資料庫單邊跨 epoch 回滾，不防同一 OS 使用者同時改檔案和 DB，也不防同 epoch 新命令尾端的回滾。
- task streams／ingress／Control product 等已實作的固定資料閉包一起刪除。未知表、尚未支援的資料種類及跨範圍引用會列入 blockers；不可把 blocker 當成已清除。預覽的 counts 是實際範圍，並非所有未來擴充功能的涵蓋承諾。
- SQLite：只接受既有精確 schema profile；工作、事件、內部關係、登記、觀察及其附表在交易中清除。名單已先被移除時，必須由當次 PostgreSQL 預覽提供相同 ID、路徑與 scope digest，才能接手殘留資料；名單不存在本身不能充當清除成功。其他專案及所有保留資料的完整內容摘要必須不變；同一路徑的其他專案登記仍會阻擋刪除。
- 目標有 installation receipt 時，Windows 自動採 `REBUILD_SURVIVORS_V1`：不修改或停用原資料庫的 append-only trigger，以精確原 schema 重建保留資料。逐項核對原 row、rowid、事件序號高水位、完整性、FK、索引、trigger 與檔頭設定，再同目錄替換舊檔。重建前以系統 Windows PowerShell 唯讀檢查 owner/group/DACL，必須與新暫存檔完全相同；自訂 ACL、無法讀取或中途變動都拒絕，不能為通過清除而放寬權限。SACL 稽核設定未驗證；此版未提供權限複製或非 Windows 的重建轉接器。一般開啟會恢復 WAL 模式；SQLite schema 仍為相容的 v7。
- decisions 的 scope 是自由字串，沒有專案 FK。偵測到目標引用時維持阻擋，所有 decision 與 decision_state 原文保留；沒有找到 ID／路徑也不能證明沒有相關決策。報告另列 `LEGACY_SCOPE_OWNERSHIP_NOT_PROVEN`，需要由正常決策模組提供結構性歸屬後才能處理。不能用字串猜測刪掉整條決策歷史，也不以新增 SQLite epoch 掩蓋歸屬未知。
- 檔案：只接受權威清單中的絕對路徑；拒絕使用者家目錄、磁碟根、工具自身、其他專案、重疊根、祖先 junction、未支援的巢狀 repository 等。junction／symlink 僅移除連結，不追蹤目標。檔案預覽有數量、深度、manifest 大小上限。
- 硬連結：目前在盤點及執行前驗證時拒絕 `nlink > 1` 的檔案或連結，即使所有名稱看似位於同一專案。移除其中一個名稱會改變共用檔案的連結數與時間戳；本版沒有完整的硬連結歸屬與續作轉接器，因此須在任何 PostgreSQL／檔案刪除前阻擋，不能先刪一半再卡住，也不能略過時間戳驗證或改動外部連結以強行通過。
- Control 圖譜磁碟快取：從共用快取目錄盤點全部直接子目錄，以 graph.json 格式、project_id、source_root、目錄鍵及內容摘要建立歸屬，涵蓋同專案不同 checkout。只有確定屬於目標的快取才加入相同檔案清單；內容變更、新增目標快取、未知目錄、缺失標頭、連結及超出盤點上限均阻擋。共用快取目錄與專案刪除根重疊也阻擋，以保護其他專案快取。此項不涵蓋 Runtime 的共用 Graphify 記憶體／索引或仍運作的 Control 記憶體快取，執行仍需停止相關寫入者。
- 這不是磁碟安全抹除：SQLite／PostgreSQL 的備份、WAL、儲存媒體殘留及外部副本不由本入口保證消失。它驗證的是支援範圍的邏輯資料與路徑不再存在。

Runtime Graphify 的檔案範圍可由 `runtimeGraphWorkDirectory` 或既有 `LATTICE_GRAPHIFY_WORK_ROOT` 指定。原生程式以 Registry 路徑重用 Runtime 的 canonicalize 與相同 domain hash，將 `sources/<source key>` 加入固定刪除清單；其他 source 目錄保留。只在此已綁定範圍允許空的巢狀 `.git` 快照邊界，含任何 Git metadata 的目錄仍拒絕。未知舊平鋪格式、別名、無法核對的來源均阻擋。未配置 work root 不代表没有快取。主 Store 的實際 Memory 表另行唯讀盤點；有舊 analysis 時仍須證明其歸屬，不能把固定的 `task032-delivery` 當成 Registry ID。

Bot lifecycle 使用**獨立 PostgreSQL cluster**，安裝器禁止與 Store 共用服務。協調層讀取既有 `%USERPROFILE%/AppData/Local/LATTICE/bot-lifecycle-postgres/v1/identity.json` 的 port、runId、systemIdentifier，或接受同形的 `botService` 設定（不含密碼）。原生端唯讀比對服務 system identifier、拒絕主 Store cluster，再驗證專用 DB schema 與 roles/events；缺失配置、連線失敗及 schema 不符都維持未知。只有這個已核對服務內的不存在／空資料庫可以標示無資料，不代表所有其他主機或服務都不存在。相同或不同文字 project_id 都不足以證明 Registry 歸屬，有資料時保留未完成。

## 執行入口

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

`externalResources` 可省略，僅接受 `kind`、`reference`、`source`。七種分類為 `codex`、`automations`、`git`、`graphify`、`botLifecycle`、`backups`、`maintenance`。這個歷史欄位名稱為相容而保留，不代表七類皆是外部：報告的 `ownership` 區分 LATTICE、Codex 及混合範圍，LATTICE 未完成項標成 `PRODUCT_WORK_REQUIRED`，並列出具體下一步。沒有填資源不等於不存在。呼叫者不能自行填入已刪除、已驗證或完成狀態來取得通過；`latticeScopeComplete` 與全域 `complete` 分別保留，兩者目前都未達成。

`codeGraphCacheDirectory` 可省略，預設與 Control 相同：`%LOCALAPPDATA%/LATTICE/control/code-graphs`。若 Control 使用自訂目錄，必須提供實際目錄。報告將這個可驗證磁碟範圍獨立列為 `controlCodeGraph`；即使通過，外部 `graphify` 分類仍未完整驗證。沒有此盤點欄位的舊計畫保持原範圍，不能據此聲稱已清掉快取。

使用最小驗證憑證前，以原生入口的 `install-epoch` action 與 `INSTALL_REGISTRY_EPOCH_MAINTENANCE` 授權安裝精確的可選 catalog；仍須已停止 Store。它包含一般清除 receipt schema，不改 admission 或角色權限。舊 Runtime 不支援此新增 catalog，須先準備相容讀取者；schema 安裝不是 Runtime 部署。安裝完成後重新產生預覽，核對 `history` 的 redactedSurvivorCommands、assurance 與限制，再確認同一計畫。

每次輸出均包含同一格式的清除報告。`apply`／`resume`／`status` 在本機範圍完成時 exit 0，但報告仍為 `PARTIAL`、`complete: false`；`verify` 對這種情況回傳 exit 2。阻擋或錯誤回傳 exit 1。由於外部驗證尚未實作，本版 `verify` 不會回傳完整通過，任何自動化均不得用 `apply` 的 exit 0 宣稱整個專案清空。

## 交易、部分失敗與續跑

1. 以獨占 operation lock 防止同一進度檔同時執行；一般清除在 SQLite `BEGIN IMMEDIATE` 內核對原始摘要並保持鎖。收據重建則先建立資料庫旁的 `.purge-swap` 維護標記、完成 WAL checkpoint 並切換 DELETE journal，再持有 `BEGIN EXCLUSIVE`；新 Control 在開啟資料庫前見到維護標記會拒絕。
2. 先核對檔案 manifest、PostgreSQL scope digest／blockers，再執行 PostgreSQL 清除交易。
3. 以 operation ID 與 scope digest 讀回 PostgreSQL receipt，接著逐項移除檔案；最後才提交 SQLite 清除。
4. 三個儲存系統無法組成單一原子交易。中斷、鎖檔、權限錯誤或回覆遺失都記為 `INCOMPLETE`；不能宣稱已回復原狀。已提交的 PostgreSQL 清除不能由 SQLite rollback 撤銷。
5. 用原計畫、原 operation ID 與進度檔續跑。PostgreSQL 回覆遺失時先讀回 receipt，不再次刪除。每個檔案移除前先寫入 intent 並同步進度，移除後再保存結果；若在兩者之間崩潰，續作只會對原計畫中那一項的缺失進行核對。未知路徑、內容變更及預覽以外的變動仍會拒絕。歷史 receipt 還要符合目前資料的 `afterDigest`；即使是保留專案後續合法變更，也須重新核對，不能只憑舊成功紀錄宣稱目前已清空。
6. 若程序異常留下 `.lock`，先確認該 operation 已無執行程序並讀回所有階段，再處理鎖檔；程式不會自動猜測 stale lock。不得刪除未知鎖或啟動第二個 writer。
7. 三階段讀回通過只回報 `SCOPED_PURGED`。`externalCleanup: NOT_VERIFIED` 明確保留外部清理待辦；只有外部逐項驗證也完成，才可向使用者說「整個專案已清空」。

Attested Registry 在 PG 刪除前先寫外部 `Pending(previous, next, operationDigest)`，再用單一 PG 交易寫新基準、ID 防重表與清除收據。PG 提交後，必須比對同一新基準及已提交收據，才將主機憑證改為 Active。正常 Runtime 遇 Pending 一律拒絕。提交前中斷只能沿用同操作／摘要驗證 previous；提交後中斷只能以匹配的收據完成 next，不可用「舊狀態驗證失敗」猜測已提交。缺少主機憑證、兩邊不符或未知鎖都維持阻擋，不從 DB 自動重建。人工處理時先停住所有 reader/writer、保存原錯誤、核對原計畫與兩邊實際狀態；沒有独立可信證據時不能補造新憑證或重新開始刪除。

這裡的中斷續作指程序崩潰或一般 I/O 失敗；不保證突然斷電後可自動續作。進度檔雖先做檔案同步再原子替換，但父目錄項與目標檔案刪除的斷電落盤順序尚未驗證，尤其 Windows 不能由目前 Node API 假定相同保證。重啟後若進度與檔案不符，維持阻擋並人工核對，不補造已移除紀錄。

SQLite 重建在 PG 與檔案成功後才填入 `.purge-next` 保留資料檔；關閉來源與暫存資料庫後，兩側的 WAL／SHM／journal 必須不存在，才保存綁定原計畫的新舊檔摘要並替換。換檔前失敗可從原檔與暫存檔繼續；換檔後但移除標記前失敗，只有新檔摘要正確且暫存檔已消失才接受完成。未知檔案、缺失 DB、錯誤計畫、旁檔、連結或內容變動一律阻擋，不補造新資料庫。未完整落盤的 marker 暫存檔、尚未登記身分的 staging、或中斷重建留下的 journal 需要先依原計畫人工核對，不能任意刪除來解鎖。成功時不保留原 DB 副本；OS 區塊、備份及稽核設定仍不在抹除保證中。舊客戶端不識別此標記，維護前必須停止全部寫入者；沒有宣稱已排除所有 OS handle。

計畫／進度檔本身保留路徑及清除證據；它們也是最終資料保留決策的一部分。不要將實際專案計畫、資料庫或含機密的測試輸出提交 Git。

## 套件交付

`scripts/lattice-bundle.py build` 可用三個成組參數加入清除功能：`--project-purge-binary`、`--project-purge-sha256`、`--project-purge-source`。最後一項是提供本版 Node 依賴檔案的 repository 根目錄。三者必須同時提供；打包及驗證會核對固定的 binary／CLI 檔案清單與 SHA-256，並將 `project_purge` 能力綁到同一套件的 Runtime hash。套件內直接使用 `node apps/lattice-control/src/project-purge-client.mjs --help` 查看入口。

此能力仍須明確加入套件；舊套件維持相容，不會因 repository 新增檔案而自動取得刪除功能。套件驗證也不會解除資料庫、共用 Git、交錯歷史或工具政策的阻擋。

只需要本機離線維護工具時，可用 `build-maintenance`，參數為 `--bundle`、`--runtime`、`--runtime-sha256`、`--node`、三個 purge 參數，以及 `--vc-redist`／`--vc-license`／`--vc-redist-list`。固定清單包含相容 Runtime、清除 binary、Node/CLI 依賴閉包、VC runtime 及授權來源；不重複複製 PostgreSQL、Python、Git 或 Graphify。用 `verify-maintenance --bundle ... --sha256 ...` 核對。這是本機候選維護包，不能交給一般 `install` 假裝完整依賴部署。

## 驗證

```powershell
node --test apps/lattice-control/test/project-purge*.test.mjs
cargo build -p lattice-postgres-store --example project_purge_fixture
cargo build -p lattice-runtime --bin latticed --bin lattice-project-purge
cargo build -p lattice-runtime --example project_purge_epoch_fixture
pwsh -NoProfile -File scripts/test-project-purge-postgres.ps1 -PurgeBinary <lattice-project-purge.exe 絕對路徑> -SeedBinary <project_purge_fixture.exe 絕對路徑> -RuntimeBinary <latticed.exe 絕對路徑>
```

PG harness 使用新的 loopback cluster、合成專案與任務，保留 `.lattice` 下的證據，不連正式資料庫。Windows 檔案測試包含實際鎖檔及部分失敗續跑。協調層測試使用真 SQLite／檔案與受控 native adapter；真 PG fixture 的結果須另外報告，不能把 mock 當成完整部署驗收。

`-Scenario inventory` 可單獨驗證運作中／未安裝維護元件的盤點不改動資料、離線要求仍生效，以及狀態改變後的舊摘要被拒絕。

`-Scenario bot-inventory` 使用全新且沒有 Store 的獨立 cluster，經真實安裝器和正常 Bot register API 寫入合成憑證，驗證不存在、空資料庫、有資料、文字 ID 不符，以及服務身分不符；不讀取使用者的 Bot 服務。`coordinator-absent` 另以真實 native source key 驗證 Runtime 快照清除、空 Git 邊界及另一專案快照原文保持不變。

`-Scenario epoch` 與 `-Scenario epoch-reference` 使用各自的新 cluster，驗證交錯歷史／跨專案拒絕紀錄、外部憑證、原始持久化收據精確重送、兩次清除及新命令。Pending 案例由 fixture 寫入真實預覽綁定的提交前／提交後檔案狀態，驗證後續程序恢復；這是中斷狀態模擬，不是突然斷電驗收。`project_purge_epoch_fixture` 使用 Runtime 的實際原生檔案識別及 Store，要求合成資料目錄 marker 和 fixture opt-in，且拒絕已知正式埠；不會加入交付套件。

`-Scenario survivor-reference` 以正式 Registry API 建立跨專案的 duplicate-denied 憑證，驗證盤點回傳引用筆數、清除被拒絕、資料及 receipt 不變、另一程序仍能重播原歷史。`-Scenario coordinator-absent` 另涵蓋真 PostgreSQL／SQLite／檔案及不同 checkout 的 Control 圖譜快取清除，並核對保留專案快取原文不变。

`-Scenario upgrade -LegacyPurgeBinary <舊版維護 binary 絕對路徑>` 使用真正舊版 binary 產生 v1 摘要與清除 receipt，再以新版 binary 讀回及重試原操作，驗證相容性。舊、新 binary 都會複製並核對 SHA-256；未提供舊版 binary 的一般測試不包含此驗證。
