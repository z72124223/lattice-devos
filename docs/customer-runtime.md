# 客戶獨立 Runtime（Windows 開發階段）

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

尚待完成：完整依賴封裝與可信版本清單、Runtime 本體的更新／回復、一般程式
關係 MCP 查詢、獨立乾淨作業系統及企業嚴格權限環境驗收。
目前只固定所列執行檔與安裝腳本，尚未證明 PostgreSQL／Python 的完整依賴樹
可攜性。DPAPI 綁目前 Windows 使用者，不能當跨使用者／跨電腦恢復方案。
此階段不代表已發布、全平台支援或三核心商品驗收完成。

## 聚焦驗證

```text
python -m unittest discover -s scripts -p test_lattice_customer_runtime.py -v
python -m unittest discover -s scripts -p test_lattice_mcp_config.py -v
cargo +1.97.1 test -p lattice-runtime --lib project_bridge::tests
cargo +1.97.1 test -p lattice-contracts --test task_ingress_contracts
python scripts/verify-lattice-customer-restart.py --state <隔離安裝目錄> --project-id <正式專案ID> --completed-task <已驗證範例工作ref> --output <全新證據JSON路徑>
```

重啟驗證會先確認範例工作已有正式完成摘要、決策與父子關係，再停止及重啟
同一個已核驗 cluster，以新 MCP 程序比較前後完整快照。它不建立工作、不修改
完成狀態，也不把這個範例的完成當成整個下載版完成。
