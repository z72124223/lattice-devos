# 客戶 Codex MCP 設定接入（開發中的本機元件）

這個元件管理客戶自己的 Codex 設定，尚不是完整三核心下載安裝包。
目前寫入只支援 Windows，需 Python 3.11 以上；其他平台只可讀取診斷。
執行環境仍須另外準備並驗證 PostgreSQL、LATTICE control 和 Graphify。
不包含作者設定、記憶、技能、營運總部、多 Bot 系統或登入憑證。

## 使用

由安裝程式或協助安裝的 AI 傳入客戶明確選定的絕對設定路徑、已準備的
Runtime 執行檔及可信套件清單中的 SHA-256。下列路徑都是佔位符，不是預設值：

```text
python scripts/lattice-mcp-config.py diagnose --config <客戶設定絕對路徑>
python scripts/lattice-mcp-config.py install --config <客戶設定絕對路徑> --runtime <已準備的執行檔絕對路徑> --sha256 <套件清單摘要>
python scripts/lattice-mcp-config.py update --config <客戶設定絕對路徑> --runtime <新版執行檔絕對路徑> --sha256 <新版套件清單摘要>
python scripts/lattice-mcp-config.py rollback --config <客戶設定絕對路徑>
python scripts/lattice-mcp-config.py remove --config <客戶設定絕對路徑>
python scripts/lattice-mcp-config.py recover --config <客戶設定絕對路徑>
```

`install` 新增一個受管理的 `[mcp_servers.lattice]` command 區塊；
`update` 只換該區塊，`rollback` 重新核對前一版執行檔內容後恢復；
`remove` 移除該區塊，保留資料、執行檔、原設定與備份。
`diagnose` 不寫檔，`recover` 只在中斷後的檔案摘要可明確核對時完成管理紀錄。
既有非本工具建立的 LATTICE 設定會拒絕接管，應保留並由其原安裝入口維護。
同一版重複安裝不新增另一個 MCP。

## 設定與權限保留

原設定位元組、註解、模型、權限與其他 MCP 都保留，沒有複製作者的 Codex home。
新增區塊之外的客戶後續編輯會保留；若客戶修改 LATTICE 區塊或新增屬於該表的
設定，工具停止更新／移除並回報衝突，不自行判斷該刪除哪些客戶設定。
工具不會改 sandbox、approval 或管理員政策，也不提升權限或修改客戶檔案 ACL。
指令沒有密碼參數、不輸出設定內容，不會啟動 Runtime 或新增服務。

設定檔旁的 `.<設定檔名>.lattice` 保存管理紀錄、鎖及不可覆寫的原始設定備份。
**備份可能含客戶秘密，必須留在客戶本機，不得放入下載包、Git、回報或公開附件。**
Windows 備份會先套用原檔的存取規則，再寫入資料。
設定写入使用 Windows 排他檔案句柄；不能取得存取權時拒絕，不借父目錄權限
繞過原檔的寫入限制。寫入保留原檔身分及安全中繼資料。

## 中斷與限制

寫入前先保存備份與 pending 紀錄，完成寫入後檢查實際內容，再保存管理狀態。
這是具備排他保護的原地寫入，**不是斷電原子交易**。斷電可能留下部分內容。
`diagnose` 在 pending 存在時優先回報 `RECOVERY_REQUIRED`，區分原內容、目標內容、
或已改變／部分內容。前兩種可用 `recover` 核對收尾；最後一種必須保留現況與
備份，由維護者比對後做明確修復，工具不會用舊備份覆蓋未知的新編輯。
僅有檔案設定成功時一律回報 `runtime_workflow: NOT_VERIFIED`。
安裝後仍需由 Codex 重新載入 MCP，再驗證真正的工作、決策、證據及關係查詢。

目前這個元件不提供 Runtime 環境變數或秘密配置，也不處理企業集中政策、
平台簽署、版本下載、PostgreSQL 資料移轉或 Graphify 系統套件安裝。
Windows 符號連結／junction 會拒絕；不是可把整個作者環境複製給客戶的工具。

## 當前依賴定位

- 現有 `scripts/start-lattice-runtime-postgres.ps1` 固定 PostgreSQL 17 路徑，
  依賴已存在的 MCP env 區塊，並把倉庫位置當 Graphify 來源；尚不適合直接作
  乾淨客戶安裝入口。保留該腳本，不在作者現有資料庫上試裝。
- `crates/lattice-graphify-adapter/src/identity.rs` 明確綁定 Ubuntu 26.04、
  Python 3.14.4、bubblewrap 0.11.1、Windows WSL launcher 及完整 payload 摘要。
  需要逐一證明客戶環境符合支援邊界，不能移除驗證或接受任意系統版本。
- 首發完整平台清單仍未驗收；本元件的 Windows 測試不代表三核心下載版完成。

## 驗證

```text
python -m unittest scripts/test_lattice_mcp_config.py -v
```

測試使用暫存客戶設定與不會啟動的合成執行檔，覆蓋新增、更新、回復、移除、
原始設定保留、後續客戶編輯、真實 Windows ACL、並行寫入拒絕與中斷。
跨程序重啟測試驗的是設定管理紀錄，不是 PostgreSQL 工作成果的重啟讀回。
無建立符號連結權限的環境會明示跳過該一項，不以降低安全限制讓它通過。
另已以獨立客戶目錄讓本機 Codex 的 `mcp get lattice --json` 讀取生成設定，
並確認移除後回到原始位元組；這只驗證 Codex 設定解析，未啟動合成測試執行檔。
首次測試的舊 `approval_policy="untrusted"` 被本機 Codex 拒絕，失敗樣本保留；
重新建立使用 `on-request` 的獨立測試設定後通過。工具本身不改寫客戶的政策值，
遇到客戶既有政策與其 Codex 版本不相容時，仍需另外診斷。

設定格式依 [OpenAI 官方 MCP 文件](https://learn.chatgpt.com/docs/extend/mcp?surface=cli)：
Codex 可在客戶或受信任專案的 config.toml 設定 STDIO command，設定完成仍須重新載入。
Windows 存取規則複製使用 [GetNamedSecurityInfoW](https://learn.microsoft.com/en-us/windows/win32/api/aclapi/nf-aclapi-getnamedsecurityinfow)
與 [SetNamedSecurityInfoW](https://learn.microsoft.com/en-us/windows/win32/api/aclapi/nf-aclapi-setnamedsecurityinfow)。
