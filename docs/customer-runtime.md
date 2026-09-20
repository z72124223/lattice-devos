# 客戶獨立 Runtime（Windows 開發階段）

一般使用者請從[讓 Codex 協助安裝](../INSTALL_WITH_CODEX.md)開始。
以下為 Codex／維護者使用的操作參考，命令中的路徑及摘要由執行者核驗後填入。

`scripts/lattice-customer-runtime.py` 在客戶指定的新目錄建立獨立 PostgreSQL
cluster、Runtime 身份及加密憑證，並把啟動器複製到安裝目錄。
目前是可驗證的本機元件，**不是已完成的三核心下載包**。
它不讀作者的 Codex 設定、資料庫密碼、記憶或營運總部。

## 環境與安裝

目前支援的實作是 Windows、Python 3.11 以上、PostgreSQL 17 工具及 Git。
Graphify 另需符合既有固定身份驗證的 WSL／Ubuntu 與 Graphify payload；
此入口不下載、升級或放寬系統依賴。所有路徑由客戶環境決定。
套件摘要與依賴核對由協助安裝的 AI 處理，不要求一般使用者自行判讀。

```text
python scripts/lattice-customer-runtime.py prepare --state <全新安裝目錄> --runtime <latticed.exe> --sha256 <可信摘要> --postgres-bin <PostgreSQL的bin目錄> --git <git.exe> --graphify-runtime <已核驗payload目錄> --graph-source <客戶Git專案根目錄> --wsl <wsl.exe>
python scripts/lattice-customer-runtime.py register-project --state <安裝目錄> --project-root <客戶Git專案根目錄> --project-name <專案名稱>
python scripts/lattice-customer-runtime.py connect --state <安裝目錄> --codex-config <客戶config.toml>
```

安裝目錄的父目錄須已存在，目標本身須尚未存在。僅新目錄會設成目前使用者
與 SYSTEM 可存取；不接管既有目錄或修改客戶現有 ACL。新 cluster 使用隨機
密碼、SCRAM、資料校驗及獨立 loopback 埠，不使用既有的 5432／58743。
DPAPI 只保護本次安裝產生的憑證；安裝身份與設定摘要一起加密綁定。
密碼不寫入 Codex 設定或命令列。

`prepare` 執行既有正式初始化／schema bootstrap；日常 `serve` 只核驗並連線，
不遷移資料。新增的 `CODEX_LOCAL_MCP` 表示實際本機 MCP 來源，沿用原有權限
檢查；一般工作接案仍是 create-only，不啟動另一套代理排程器。

專案登記先保存無權威的本機 locator。原生 Runtime 再實查 Git／實體目錄，
透過既有 Registry 命令取得 PostgreSQL 正式身份。客戶目錄遺失、變更或格式
錯誤時拒絕；不退回舊 Control HTTP 服務。這個入口不恢復已退休的產品介面。

## 使用、診斷與證據

```text
python scripts/lattice-customer-runtime.py status --state <安裝目錄>
python scripts/lattice-customer-runtime.py start --state <安裝目錄>
python scripts/lattice-customer-runtime.py stop --state <安裝目錄>
python scripts/lattice-customer-runtime.py recover --state <安裝目錄>
python scripts/lattice-customer-runtime.py graphify-preflight --state <安裝目錄>
python scripts/lattice-customer-runtime.py graphify-refresh --state <安裝目錄>
```

`start` 在啟動前核對有效監聽位址、埠與資料目錄；連線後再核對 PostgreSQL
system identifier。執行檔、啟動器或固定 PostgreSQL 設定變更時拒絕。
`recover` 是明確的初始化／bootstrap 恢復操作，僅使用已綁定的同一 cluster；
不是資料備份還原或跨機器搬移。

Graphify 只分析已提交的乾淨程式版本。`graphify-preflight` 成功只代表固定
執行環境身份符合，`graphify-refresh` 回傳實際持久化收據與筆數。
現有 refresh 仍使用固定驗收查詢；其結果為零不能宣稱一般程式關係查詢完成。

來源含未提交檔案時會回報 `LATTICE_GRAPHIFY_SOURCE_UNCOMMITTED`。這是來源
版本檢查失敗，不能解讀成 PostgreSQL 權限問題。安裝器自建範例會先提交
掛勾與範例程式，再建立圖譜；不要在驗收後自動改寫使用者的來源專案。
正式功能驗收還須透過 MCP 建立／讀回驗收任務，並以正式專案 ID、相同 commit
及持久化收據讀回非空的 `lattice_code_relations` 結果。範例驗收任務只保存為
草稿，不會因安裝器檢查成功而宣稱使用者的工程任務已完成。

Codex 可使用既有 MCP 保存工作、父子／依賴關係及決策。正式本機測試成果沿
既有 operator importer 保存；新增入口不提供任意「完成」setter：

```text
python scripts/lattice-customer-runtime.py import-result --state <安裝目錄> --evidence-request <正式local-result-import請求JSON> --node <已核验node.exe> --node-sha256 <可信摘要>
```

Runtime 核對請求、專案及證據摘要，實際執行指定的 Node 測試，保留輸出並由
原有 Task Ledger 轉移工作狀態。證據請求格式沿用
`apps/lattice-runtime/src/local_result_import.rs`，不接受任意 SQL。

## 設定移除與未完範圍

連線只增量新增 LATTICE MCP；設定備份、衝突拒絕、回復及移除沿用
[設定管理元件](mcp-customer-connection.md)。移除 MCP 項目不刪 Runtime、資料庫
或證據；需要停止該安裝時另用 `stop`。既有客戶政策無法寫入時明確拒絕，
不能把權限失敗當成安裝成功。

目前已有一般程式關係查詢、可回復 Runtime 更新與本機 Windows 依賴封裝入口。
尚待完成完整跨機依賴可攜性、獨立乾淨作業系統及企業嚴格權限環境驗收。
封裝安裝會保存並持續核對完整依賴檔案集合及雜湊；未封裝的既有安裝仍只核對
原本登記的檔案。DPAPI 綁目前 Windows 使用者，不能當跨使用者／跨電腦恢復方案。
此階段不代表已發布、全平台支援或三核心商品驗收完成。

## 聚焦驗證

```text
python -m unittest discover -s scripts -p test_lattice_customer_runtime.py -v
python -m unittest discover -s scripts -p test_lattice_mcp_config.py -v
python -m unittest discover -s scripts -p test_lattice_runtime_update.py -v
python -m unittest discover -s scripts -p test_lattice_bundle.py -v
cargo +1.97.1 test -p lattice-runtime --lib project_bridge::tests
cargo +1.97.1 test -p lattice-contracts --test task_ingress_contracts
python scripts/verify-lattice-customer-restart.py --state <隔離安裝目錄> --project-id <正式專案ID> --completed-task <已驗證範例工作ref> --output <全新證據JSON路徑>
```

重啟驗證會先確認範例工作已有正式完成摘要、決策與父子關係，再停止及重啟
同一個已核驗 cluster，以新 MCP 程序比較前後完整快照。它不建立工作、不修改
完成狀態，也不把這個範例的完成當成整個下載版完成。
## Retained code relationship queries

`lattice_code_relations` requires `project_id`, an exact retained Git `commit`,
a literal case-insensitive `query` (1–128 characters), and `limit` (1–32).
An optional `task_ref` binds its usage receipt to an existing task in the same project;
PostgreSQL verifies that association. Omission remains explicitly `UNBOUND`.
It searches symbol names, relationship endpoints, relationship names and source paths.
For example, query `normalize_name` can return `greeting() calls normalize_name()`.
The Runtime requires an active registered project, selects its physical root from
PostgreSQL Registry and replays that project's original source receipt for the commit.
Missing analysis is an error; this read never runs Graphify or silently selects another commit.

Results remain `DERIVED` / `CANDIDATE` / untrusted. The original source receipt is
an anchor for the retained analysis, not acceptance evidence for the new query.
Each call also appends a separate usage receipt, so the complete tool is not read-only
or idempotent. `lattice_graph_usage(project_id, task_ref)` reads observed counts,
returned records and inner JSON bytes without starting analysis. No observations means
`UNKNOWN`, not zero use throughout a Codex task. See [usage evidence](graphify-execution-evidence.md#6-後續實作由-runtime-產生使用紀錄).

## 接入其他本機專案

安裝範例只用於安裝驗收。Codex 接到使用者的實際工作後，以安裝內附的 Python
及 `lattice-customer-runtime.py` 執行下列步驟；不必再建立另一套資料庫：

1. `register-project --state <安裝目錄> --project-root <Git根目錄> --project-name <名稱>`
   取得或重用本機 `project_id`。
2. 使用該 ID 透過 `lattice_task_submit` 提交使用者真正要求的工作。這一步才會
   由原生 Runtime 核對 Git 並登記至 PostgreSQL；已有任務時應延續，勿重複送件。
3. `graphify-refresh --state <安裝目錄> --project-id <同一ID> --task-ref <任務回傳值>` 分析該專案。
4. 確認 `operation_evidence.status=PERSISTED`，再用相同 ID、回傳的 commit
   及 `task_ref` 呼叫 `lattice_code_relations` 讀回結果，並用 `lattice_graph_usage` 核對使用紀錄。

命令會核對正式 Registry 中的專案位置；不接受只修改本機 locator 的替代來源。
每個來源有獨立圖譜工作目錄與收據選擇，原本封存的安裝來源設定不變。
全域 Codex 規則會提供本機實際命令位置，並保留原有啟動規則。

此流程要求已提交、乾淨且有分支的 Git 專案；不得為分析而擅自清除或提交使用者
變更。Graphify 分析失敗時應如實回報，不能用範例圖譜代替客戶專案結果。
非預設專案搬移位置或還原到另一目錄後，舊圖譜可能須重新分析；目前不承諾
所有專案的歷史圖譜都能直接在新位置讀回。正式工作事實仍由 PostgreSQL 保存。
The Runtime verifies the complete record-ID/content-digest set against that receipt,
then verifies every returned record's content and membership before discarding the
internal proof. This establishes returned-record integrity, not exhaustive recall or
the content integrity of records outside the returned page. `truncated` identifies
a result page with additional matches. Current working-tree changes are outside an
explicit historical-commit query.

The additive Control query function is installed only by explicit native
`--postgres-bootstrap`. Ordinary startup accepts the exact old or new catalog;
the new query reports `CODE_RELATIONS_UPGRADE_REQUIRED` on the old catalog.
Historical Control SQL, Memory identity, analyses, receipts and retrieval audits
are preserved. Older Runtime binaries that do not recognize the new catalog cannot
serve an upgraded database; binary rollback must check schema compatibility.

`scripts/verify-lattice-code-relations.py` verifies real call-edge hits, literal
empty queries, truncation, rejected selectors, retained completed work, and identical
reads after PostgreSQL and MCP restart. When using an explicitly hash-pinned candidate
binary, its evidence says `PINNED_CANDIDATE`; that does not establish installed-package
acceptance. Full dependency packaging and a recoverable installed Runtime update remain
separate acceptance scopes.

## 更新與中斷恢復

```text
python -I -B -S scripts/lattice-customer-runtime.py update-runtime --state <安裝目錄> --runtime <已核對的新latticed.exe> --sha256 <可信摘要>
python -I -B -S scripts/lattice-customer-runtime.py recover-update --state <安裝目錄>
python -I -B -S scripts/lattice-customer-runtime.py rollback-runtime --state <安裝目錄> --update-id <目前更新回傳的ID>
python -I -B -S scripts/lattice-customer-runtime.py reconnect --state <安裝目錄> --codex-config <原本受管理的config.toml>
```

更新會保留不可變的舊版與新版檔案，封存加密的前後設定，經正式 schema
bootstrap 與新 Runtime 實際讀回後才完成切換。中斷後日常入口拒絕啟動；
`recover-update` 先核對加密紀錄與資料庫身份，必要時安全啟動同一個 cluster，
再完成更新。它會保留未知的外部修改，不猜測應覆寫哪份內容。

`rollback-runtime` 只接受目前更新的 ID，且舊版必須能核驗目前資料庫；不相容
時保留新版與資料，不還原陳舊資料庫。`reconnect` 只更新原受管理的 LATTICE
設定，指向已安裝的新版啟動器。移除及設定回復仍走原設定管理入口。

新版 MCP 連線共用服務租約，允許多個連線與一般短操作並存；更新、回復及
停止／schema 恢復需排他租約。更新前須關閉仍使用舊版無租約啟動器的連線。

## 本機 Windows 依賴封裝

`scripts/lattice-bundle.py build` 從明確指定的軟體來源建立全新目錄，包含
Runtime／啟動器、PostgreSQL 軟體、CPython 3.12 標準函式庫及 DLL、Git 與
Graphify payload，以及固定摘要的 Node.js 24.16.0 與授權文件。可另加固定摘要的
Ubuntu 官方 WSL 映像。它保留來源授權文件，排除資料庫、Python site-packages、
Git 系統設定、使用者設定及憑證。每個檔案都有雜湊與大小；安裝時封存完整
清單，之後啟動與更新恢復也拒絕新增、遺失或遭改動的依賴檔案。

```text
python -I -B -S scripts/lattice-bundle.py supply-node --node <全新Node供應目錄>
python -I -B -S scripts/lattice-bundle.py build --bundle <全新封裝目錄> --runtime <latticed.exe> --runtime-sha256 <可信摘要> --postgres <PostgreSQL軟體根目錄> --python <CPython3.12根目錄> --git <Git軟體根目錄> --graphify <已核對Graphify目錄> --node <Node供應目錄> --archive <官方ubuntu-26.04.1-wsl-amd64.wsl> --vc-redist <正式VS2022的Microsoft.VC143.CRT目錄> --vc-license <原始Microsoft授權docx> --vc-redist-list <原始Redist.txt>
<封裝目錄>/python/python.exe -I -B -S <封裝目錄>/bin/lattice-bundle.py verify --bundle <封裝目錄> --sha256 <bundle.json可信摘要>
<封裝目錄>/python/python.exe -I -B -S <封裝目錄>/bin/lattice-bundle.py install --bundle <封裝目錄> --sha256 <bundle.json可信摘要> --state <封裝外的全新私有目錄> --graph-source <封裝外的客戶Git專案> --wsl <已核驗wsl.exe>
```

這仍是本機候選封裝。`import-result` 預設使用已安裝並封存摘要的 Node；未配置
時拒絕執行，不搜尋 PATH。Codex 客戶端與已啟用的 Windows WSL2／虛擬化仍是
外部需求。Hermes 已永久退役，不得啟用。它不是完整三核心可攜
下載版，也不代表跨電腦、乾淨作業系統或企業政策已驗收。

專用 WSL 安裝先執行以下入口，再將回傳的目錄傳給 bundle install 的
`--graphify-platform`。它只新增 `LATTICE-Graphify-<隨機ID>`，不採用、重設或
停止原有 Ubuntu／Docker，不修改全域 WSL 設定，也不自行啟用 Windows 功能。

```text
<封裝目錄>/python/python.exe -I -B -S <封裝目錄>/bin/lattice-wsl-platform.py provision --root <全新私有WSL目錄> --archive <封裝目錄>/platform/ubuntu-26.04.1-wsl-amd64.wsl --wsl <Windows系統wsl.exe> --wsl-sha256 <目前可信Microsoft簽署程式摘要>
<封裝目錄>/python/python.exe -I -B -S <封裝目錄>/bin/lattice-wsl-platform.py recover --root <同一私有WSL目錄>
```

供應入口驗官方映像完整摘要，鎖定映像直到匯入結束；全新 Linux 環境建立
密碼鎖定、UID/GID 1000、無额外群組的 `lattice` 服務使用者。每次客戶啟動器
呼叫前驗證 DPAPI 封存的平台登記、三個系統檔與帳戶檔。原生 Runtime 另驗
WSL 程式、三個系統檔與 Graphify payload，所有新 profile 命令明確選取專用
distribution／使用者。原生驗證本身不代表目前整個 rootfs 已逐檔驗證。
平台設定投影寫入中斷時，`recover` 只接受加密紀錄保存的精確前一版摘要；
未知修改會保留並拒絕覆寫。匯入尚未完成或服务使用者尚未建立时，恢復入口
不會猜測、接管或重建該環境。

舊 `Ubuntu` profile 與歷史摘要保持原值。專用 profile 使用新的配置摘要；
相同路徑下缺少新 profile 歷史分析時，唯讀查詢可讀回精確舊分析。重新抽取
一律使用選定的新 profile；驗證失敗不會退回舊環境執行。

Graphify refresh 要求來源專案的工作目錄乾淨。若專案由另一套 Git 複製，須先
核對換行設定與真實差異；不要把換行差異當成已修改的程式，也不要自動重設
或捨棄客戶變更。封裝不帶入原電腦的 Git 系統設定。

這些檢查驗證靜態檔案與安裝身份；不宣稱可抵禦同一 Windows 使用者在核對後
併發替換檔案，或已遭替換的 Python 本體在自我核對之前執行。

## 客戶資料備份與新位置還原

```text
<封裝目錄>/python/python.exe -I -B -S <封裝目錄>/bin/lattice-customer-backup.py backup --state <原安裝目錄> --output <全新備份目錄> --key-root <另一個全新私有目錄> --evidence-root <要保留的外部驗證證據目錄>
<封裝目錄>/python/python.exe -I -B -S <封裝目錄>/bin/lattice-customer-backup.py restore --state <全新還原目錄> --backup <備份目錄> --sha256 <backup.json可信摘要> --key <獨立保存的recovery.key> --bundle <已核驗封裝> --bundle-sha256 <bundle.json可信摘要> --graphify-platform <目標使用者新建且已核驗的WSL目錄>
<封裝目錄>/python/python.exe -I -B -S <封裝目錄>/bin/lattice-customer-backup.py finalize-restore --state <同一還原目錄>
```

備份會短暫停止已核驗的自有 PostgreSQL，保存資料庫、已登記的獨立 Git
專案（包含未提交檔案）、自有 graph-work／dependencies，以及明確指定的
外部證據目錄。完成或失敗後恢復原本的資料庫啟停狀態。資料以 Node 的
AES-256-GCM 加密，逐檔驗證實際讀入的內容；恢復金鑰另存於私有目錄。
保管備份、可信摘要與金鑰；跨電腦還原不依賴原 Windows 使用者的 DPAPI。

目前接受已核驗為 ACTIVE 的專案、獨立 `.git` 目錄、無外部 tablespace／
Git alternates／路徑重新導向的來源。超過 100,000 個檔案、10 GiB 明文或
32 MiB metadata 時拒絕產生成功備份。來源資料與失敗測試產物保留。

還原只寫全新私有目錄，先解密與逐檔核驗，再使用目標端可信封裝的相同
PostgreSQL binary、新 loopback port、新 DPAPI 憑證及新 WSL 平台設定。
原 run ID、cluster system ID、工作與證據摘要保留；原生 Runtime 透過
正式 Registry observe／reconcile 紀錄新實體身分，不改寫歷史資料列。
原生入口另驗 DPAPI 還原紀錄、安裝與 catalog 摘要、專案 proof、精確版本
及 pending observation。普通啟動、recover 與 MCP 在還原中會被擋住。

`finalize-restore` 只重播原已封存請求。若在首次憑證與還原 journal 建立前
中斷，保留失敗目錄並從原備份還原到另一個全新目錄；它不接管未知目錄。
成功後仍須讀回正式工作、決策、成果與 Graphify 關聯，再核對重啟一致性。
歷史 Graphify selector 會沿後續備份保留，最多 16 個；僅用於原資料庫的
唯讀歷史查詢，重新抽取仍使用新平台。兩份安裝後續的新變更不會自動同步。

已實測的同機不同目錄還原，不等於另一 Windows 使用者、另一台電腦、
乾淨 OS 或企業政策環境的驗收；這些環境仍需獨立測試。

## 可重複執行的客戶環境驗收

`scripts/verify-lattice-customer-environment.py` 是獨立驗收入口，配合已核驗的
bundle-v5 或相同介面的後續封裝，不需要更動封裝內容。先以備份還原到全新
本工作專屬目錄，再執行：

```text
<封裝>/python/python.exe -I -B -S <驗收工具>/verify-lattice-customer-environment.py --bundle <封裝> --bundle-sha256 <可信摘要> --state <已還原安裝> --output <全新證據目錄> --reference <來源工作讀回JSON> --project-id <原專案ID> --completed-task <已完成task_ref> --commit <原Graphify commit> --query <原有關聯查詢>
```

reference 使用 `after` 下含 `task`、`product`、`decisions`、`graph` 的原始
讀回 JSON。工具使用測試設定實際記錄的 MCP 命令，核對原成果、父子工作、
決策及關聯；乾淨與自訂設定各自完成 PostgreSQL／MCP 重啟核對，移除後
必須恢復原設定 bytes。它另對全新測試檔加入當前使用者的 NTFS 拒絕寫入
規則，先觀察真正的作業系統拒絕，再確認 LATTICE 拒絕修改且未建立側錄
狀態，最後還原該測試檔的原存取權。所有來源、失敗證據與客戶設定保留。

這個入口不更改真實 Codex 設定、主機帳戶或全域安全政策，也不等於已測
Codex App 的 UI、企業群組政策或另一實體電腦。報告記錄 Windows 版本與
安裝識別摘要；同一 Windows 安裝下的新目錄仍須標為同機同 OS 驗收。
獨立 Windows 客體須先有合法可使用的映像及所需註冊／授權，再在客體內
建立自己的 WSL 平台與還原目錄，執行相同入口，才能補該環境的證據。
