# 讓你的 Codex 協助安裝 LATTICE

你需要自己的 Windows 電腦、可用的 Codex App，以及可下載軟體的網路。
把下面這段話交給 Codex，它負責查環境、準備依賴、執行安裝和驗收。
你只需處理 Windows 授權、必要的重新開機，以及選擇要管理的專案。

**目前提供安全的一鍵入口，但尚未宣稱任何新電腦都能完成安裝。**
`Install-LATTICE.cmd` 會先做唯讀預檢，只有完整 bundle、Graphify、WSL 啟動器與
雜湊驗證都通過時才會建立客戶 Runtime。v2.0.0 尚未提供完整已核驗的公開
Graphify payload，因此缺件時會安全停止並寫出報告，不會假裝成功。

一般使用者使用方式（先以唯讀模式檢查，再安裝）：

```text
Install-LATTICE.cmd <Graphify-source-folder> <WSL-launcher>
Install-LATTICE.cmd <Graphify-source-folder> <WSL-launcher> --install
```

報告會寫到 `%LOCALAPPDATA%\\LATTICE\\one-click-report.json`。這個入口不會下載
未固定版本的依賴，也不會覆寫既有 Runtime 或刪除使用者資料。

Graphify 的公開 wheel 可由維護者用以下入口取得並驗證；它不能取代尚未公開
的專用 WSL 映像：

```text
python scripts/lattice-graphify-supply.py --output <全新Graphify供應目錄>
```

Ubuntu 26.04.1 WSL 映像也有 Canonical 公開來源與固定摘要，可由維護者用以下
入口下載；下載約 400 MB，完成後才能建立真正可攜的三核心 bundle：

```text
python scripts/lattice-wsl-supply.py --output <全新目錄>\\ubuntu-26.04.1-wsl-amd64.wsl
```

## 複製給 Codex

```text
請協助在我的 Windows 電腦安裝 LATTICE 三核心：控制、PostgreSQL、Graphify。
請先讀取 https://github.com/z72124223/lattice-devos/blob/product/lattice-control-mvp/INSTALL_WITH_CODEX.md
及其中引用的安裝文件，核對目前發布資產與支援範圍，再執行。

沿用我目前選擇的 Codex 模型、帳號與權限。不要使用作者的設定或憑證。
先檢查相容性和 Graphify 已核驗依賴是否能取得，再下載缺少的必要元件。
依本文件從官方來源取得依賴；不要要我自己找路徑、摘要或貼一串終端機指令。
我授權本次安裝必要的專用本機目錄、依賴、資料庫及 LATTICE MCP 設定；
保留我的其他 MCP、模型、技能、記憶、既有資料庫與 WSL 環境。
Hermes 已永久退役，不得安裝或啟用。不得改寫產品程式或放寬核驗來讓安裝過關。
涉及 Windows 提權、啟用虛擬化、重新開機或既有 LATTICE 設定衝突時，
說明必要動作並讓我處理；不要自行重開機或關閉其他工作。
遇到缺少公開依賴、政策限制或額度不足，保留可接續進度並如實說明，不能宣稱成功。
完成後請實測三核心、完成一筆測試工作並重新連線讀回結果，用白話回報。
```

## 給執行安裝的 Codex

### 先核對，不依賴作者電腦

1. 讀取本文件、[客戶 Runtime 操作](docs/customer-runtime.md)及所用腳本的 `--help`。
   第一次安裝尚無 LATTICE MCP 時，記為未安裝並繼續環境檢查；不能假造 Runtime 狀態。
2. 確認 Windows x64、可用磁碟、Python／Git、WSL2／虛擬化與 Codex 本機操作能力。
   本入口以 Windows 11 x64 為優先驗收環境，其他平台不視為已支援。
3. 核對 [Release](https://github.com/z72124223/lattice-devos/releases/tag/v2.0.0)
   的實際資產。Runtime 為 `latticed-2.0.0-windows-x64.exe`，其 SHA-256 是
   `46f45b980f6f7ce46cb1448fe1baa4e81f0094c037efb41165752548df14117c`。
   下載後與此摘要及發布頁 `SHA256SUMS.txt` 比對。不執行摘要不符的檔案。
4. 同時取得 v2.0.0 的原始碼（含完整 `scripts` 目錄）。不要混用未知分支的安裝腳本；
   後續版本應採該版公開的執行檔、腳本、依賴清單與摘要。
5. **先確認 Graphify 供應可行，再進行大型下載／Windows 功能變更。**
   通用 `pip install graphify`、一般 Ubuntu 或上游同名版本不能替代 LATTICE 的固定
   payload、SQL 擴充、平台身份與 manifest 核驗。v2.0.0 尚無公開完整依賴資產；
   若也沒有已核驗的本機來源，回報「缺少相容 Graphify 依賴，三核心安裝待完成」。
   不複製作者的已安裝 WSL、資料庫或私人目錄，不修改預期摘要來接受任意下載。

### 依賴來源與版本

| 元件 | 來源及處理方式 |
|---|---|
| PostgreSQL | 從 [PostgreSQL Windows 頁](https://www.postgresql.org/download/windows/)連到官方列出的供應來源，選 17 系列工具；使用專用資料目錄，不接管現有 cluster。 |
| Python | 從 [Python Windows 頁](https://www.python.org/downloads/windows/)取得 CPython；現有啟動腳本要求 3.11+，依賴封裝流程使用 3.12。不要用任意最新版替換固定封裝設定。 |
| Git | 從 [Git for Windows](https://git-scm.com/install/windows)取得；保留使用者的全域設定，不自行改寫換行或憑證設定。 |
| Node | 使用 `scripts/lattice-bundle.py supply-node`，它下載並核驗固定 Node 24.16.0；不要手工改動腳本的預期摘要。 |
| WSL2 | 依 [Microsoft WSL 文件](https://learn.microsoft.com/en-us/windows/wsl/install)檢查和引導啟用。可能需要管理員授權與重開機；不重設、移除或停止使用者現有 Ubuntu／Docker。 |
| Graphify | 只接受 Runtime 核驗的 payload 與平台。專用 WSL 路徑使用 `lattice-wsl-platform.py`；完整要求見客戶 Runtime 文件，缺少來源就保留未完成狀態。 |

記錄實際來源、版本、上游提供的簽章／摘要及核驗結果。不把「自己算出的 SHA-256」
當作下載來源可信的證明。不要下載其他人的私人 config、認證檔或整份使用者資料。

### 使用現有安裝入口

以下命令是給 Codex 填入本機實際路徑的範本，不需要使用者手動填寫。
每一步確認退出碼與輸出；失敗時停止後續相依步驟。`STATE` 必須是全新的專用目錄，
父目錄已存在；若已有 LATTICE，先核對身份與狀態，使用既有 recover／update 入口。

```text
python scripts/lattice-bundle.py supply-node --node NODE_SUPPLY
python scripts/lattice-customer-runtime.py prepare --state STATE --runtime RUNTIME_EXE --sha256 RELEASE_SHA256 --postgres-bin PG_BIN --git GIT_EXE --graphify-runtime VERIFIED_GRAPHIFY --graph-source PROJECT_ROOT --wsl WSL_EXE --node NODE_EXE
python scripts/lattice-customer-runtime.py register-project --state STATE --project-root PROJECT_ROOT --project-name PROJECT_NAME
python scripts/lattice-customer-runtime.py graphify-preflight --state STATE
python scripts/lattice-customer-runtime.py graphify-refresh --state STATE
python scripts/lattice-customer-runtime.py connect --state STATE --codex-config USER_CODEX_CONFIG
python scripts/lattice-customer-runtime.py status --state STATE
```

`VERIFIED_GRAPHIFY` 必須已符合該 Runtime 接受的身份；專用 WSL 平台改用文件中的
`--graphify-platform` 流程，不能把任意 Ubuntu 路徑代入。
使用者還沒有專案時，先詢問專案或取得建立本機示範 Git 專案的同意。
分析要求乾淨的已提交版本；不捨棄現有變更來通過檢查。

`connect` 使用目前登入使用者的 Codex 設定位置，只增加受管理的 LATTICE 連線。
先保留設定備份，核對變更範圍；發現同名既有連線時，不覆蓋未知設定。
讓使用者重新連線後，檢查 **Codex 自己的 MCP 工具**確實可用。
不要將作者本機的 `LATTICE_DELIVERY_*`、帳號或絕對路徑貼進客戶設定。

### 驗收與接續

- 原生 `lattice_runtime_status` 成功，使用 `GRAPHIFY`；沒有 Hermes 啟動或反思工作。
- `graphify-preflight` 與實際 `graphify-refresh` 成功，關係查詢可讀回指定提交的資料。
  `PREPARED` 或 `CONFIGURED_NOT_VERIFIED` 只代表設定存在。
- 在使用者選定的示範專案建立一筆小型驗證工作；由 Codex 執行，透過
  [本機結果匯入](docs/customer-runtime.md)的 `import-result` 實際執行測試與記錄結果。
  證據格式以 `apps/lattice-runtime/src/local_result_import.rs` 為準，不能直接寫 SQL 改成完成。
- 關閉本次測試連線後，以新連線讀回相同工作、結果摘要與必要關係／決策。
  客戶重啟驗收腳本需要已完成的自有 fixture；不要拿空白資料庫就宣稱通過。
- 列出本機安装位置、已完成與未完成項目、必要下一步。遇到重開機或額度不足，
  只在本機保存不含秘密的短接續紀錄；不要預設建立無限心跳或更換模型。

只有全部相關驗收完成，才能說「這台電腦的三核心安裝完成」。下載成功、程序存在、
MCP 名稱出現或舊版測試報告都不能替代實測。

## 目前仍需補齊

公開完整 Graphify 依賴的供應／重建流程，以及無作者環境的乾淨電腦驗收。
本入口讓 Codex 接手技術操作，並沒有消除這兩項產品缺口。
安裝協助仍使用客戶自己的 Codex 額度；本文件不是完整安裝包或跨平台相容保證。
