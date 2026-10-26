# 一件件：繁體中文本機待辦清單

使用 Node.js 標準函式庫，無需安裝套件、資料庫或建置工具。已在 Node.js 24.16.0 驗證；請使用 Node.js 24 或更新版本。

## 啟動與使用

在工作目錄開啟終端機執行：

```powershell
node todo-web/server.mjs
```

瀏覽器開啟 **http://127.0.0.1:3000**。也可先 `cd todo-web` 再執行 `node server.mjs`。

若瀏覽器仍開著舊版頁面，請先以啟動訊息中的網址重新載入首頁，再送出表單。

1. 在「新增待辦」輸入內容，按「新增待辦」或 Enter。
2. 按待辦旁的「標記完成」，完成項目會移到清單後方。
3. 按「重新開啟」，即可回到待完成狀態。
4. 重新整理頁面會載入已保存資料，不會重複新增成功的項目。
5. 終端機按 Ctrl+C 停止；下次啟動會保留內容與完成狀態。

空白、只有空格或不可見空白的內容會被拒絕；每件最多 200 字。網頁與伺服器都會檢查輸入，錯誤時顯示繁中提示。網頁不需要 JavaScript，手機窄螢幕也能操作。

若 3000 已被其他程式使用，可在 PowerShell 指定其他連接埠：

```powershell
$env:PORT = '3100'
node todo-web/server.mjs
```

之後開啟 **http://127.0.0.1:3100**。程式固定只監聽 `127.0.0.1`，PORT 僅改變連接埠，不能開放區域網路或外網存取；請使用啟動訊息所列的精確網址。

## 資料保存

- 預設資料檔固定為 `todo-web/data/todos.json`，與啟動時的所在目錄無關；首次成功新增時自動建立。
- 每次成功變更先完整寫入暫存檔，再替換資料檔；保存成功才回應成功。單一程序內的同時操作依序保存。
- 這是單機、單一程序使用的工具。**同一資料檔請只啟動一個程序**，資料不會上傳或跨裝置同步。
- 要備份時先停止程式，再複製該資料檔。若檔案損壞，啟動會失敗並保留原檔，請從備份還原；不會自動清空資料。
- 可用 `TODO_DATA_FILE` 環境變數指定另一個完整資料檔路徑；通常不需設定。異常斷電的硬體保存保證不在本工具驗收範圍。

## 固定測試程序

在工作目錄執行：

```powershell
node --test todo-web/test/todo.test.mjs
```

或在 `todo-web` 內執行 `node --test test/todo.test.mjs`。測試使用 Node.js 內建測試框架、真實本機 HTTP 請求、測試專用資料目錄和獨立 Node 子程序，不會使用日常資料。暫存資料位於 `todo-web/.test-tmp/`，每次測試後移除自己建立的子目錄。

涵蓋網頁與樣式、本機綁定、新增／完成／重開、重新整理、程序終止與重啟、空白與超長輸入、錯誤欄位與狀態、找不到項目、不支援的格式／方法、過大請求、跨來源拒絕、文字安全顯示、同時新增、損壞檔保留、保存失敗及恢復。

另涵蓋同源表單標頭的新增／完成／重開、每種回應的 Referrer-Policy，以及 null、缺少／錯誤 Origin、同站不同來源、跨站／矛盾 Fetch Metadata、錯誤 Host 的拒絕與資料不變。Node 測試明確構造 HTTP 標頭；真正瀏覽器產生的標頭另以瀏覽器操作驗收，不能由 Node fetch 測試代替。

本次修復的測試、瀏覽器 HTTP 紀錄及獨立驗收保存在 `evidence/repair/`；`evidence/` 原有檔案是複製保留的舊成果，不能當成本次通過證據。這些只證明列出的本機行為，沒有發布或合併。

## 表單來源修復

原版回應 `Referrer-Policy: no-referrer`，會讓一般 HTML 表單 POST 帶上 `Origin: null`，即使 `Sec-Fetch-Site` 仍是 `same-origin`。舊測試一律自行指定正確 Origin，因此沒有發現瀏覽器表單被拒絕。[WHATWG Fetch 的 Origin 演算法](https://fetch.spec.whatwg.org/#append-a-request-origin-header) 說明此行為。

本版改為 `Referrer-Policy: same-origin`，讓同源表單送出可核對的 Origin；跨來源仍不送 Referer。[Referrer Policy 標準](https://w3c.github.io/webappsec-referrer-policy/#referrer-policy-same-origin)

寫入必須有精確符合啟動網址的 Origin；`null`（包含搭配 `same-origin`）、缺少或錯誤 Origin 都回應 403，不能靠 Referer 放行。若有 `Sec-Fetch-Site`，必須也是 `same-origin`；`same-site`、`cross-site`、`none`、空值或未知值都拒絕。沒有 Fetch Metadata 的用戶端仍必須提供精確 Origin。Host、只監聽本機、輸入檢查與原有安全標頭一併保留。

ROOT 已於 2026-09-05 使用 Codex 內建瀏覽器，在測試網址 `http://127.0.0.1:51974/` 實測通過：新增一筆、標記完成並重新整理、重新開啟並重新整理，以及送出三個空白後顯示錯誤且維持一筆資料。最後保存的項目為「完成 LATTICE 端到端驗收」，狀態為待完成。這是 ROOT 的實測結果；本次補正核對並保存其證據，沒有另行重跑瀏覽器或 Node 測試。

[ROOT 驗收原始副本](evidence/repair/root-browser/formal-browser-acceptance-root.json)、[HTTP 請求原始副本](evidence/repair/root-browser/todo-browser-fixed-origin-evidence.jsonl)、[資料檔副本](evidence/repair/root-browser/todo-browser-fixed-data.json) 及[來源與雜湊核對](evidence/repair/root-browser/provenance.json) 均已保存。紀錄中的 task_ref、server.mjs SHA-256、HTTP 與資料檔 SHA-256 全部符合；4 次 POST 的 Origin 均為正確本機來源，Sec-Fetch-Site 均為 same-origin。HTTP 紀錄只包含請求，不含回應狀態或標頭；操作畫面結果依 ROOT 驗收紀錄，截圖留在該 ROOT 對話中，未提供獨立圖片檔。

目前瀏覽器驗收狀態為 **通過（ROOT 實測）**，見[本次驗收紀錄](evidence/repair/acceptance.json)。原先的 NOT RUN 與等待狀態完整保存在 [歷史驗收紀錄](evidence/repair/history/acceptance-before-root-browser.json)，補正前說明也保留副本；原獨立審查紀錄維持原樣，反映當時尚未收到 ROOT 瀏覽器證據的狀態。程序重啟、跨站拒絕與損壞／寫入失敗保存行為仍由原固定 Node 測試證據支持。
