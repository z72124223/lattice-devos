# 專案清除流程

這是離線維護入口，清除指定專案在已支援資料庫及檔案範圍內的實際資料。它不是封存、隱藏名單或刪除所有外部系統紀錄的保證。一般 Control API 沒有新增遠端刪除入口。

## 先確認範圍

1. 以 PostgreSQL Registry 的專案 ID 與 canonical path 核對 Control SQLite；名稱相同不代表同一專案。
2. 盤點任務、執行中工作、租約、其他專案引用、登記路徑、worktree、junction、共用 Git 目錄，以及備份與外部資源。
3. Codex 對話／附件、排程、遠端 repository／發布物、備份、獨立 Bot lifecycle DB 和外部 Graphify cache 另行逐項處理並讀回。封存對話不能填成永久刪除；沒有可用刪除介面時維持未完成。
4. 使用者確認的清除範圍必須包含永久刪除。若某次執行被工具政策拒絕，停止該操作並保留錯誤；不得改用本入口或其他工具重試以繞過拒絕。

## 支援範圍與拒絕條件

- PostgreSQL：在既有 Store `STOPPED` 維護狀態，以專用 migrator 連線執行。Registry 目前只支援目標專案命令構成全域歷史最後一段的情形，清除後恢復保留前綴的原 checkpoint，再用既有完整重播驗證器讀回。交錯歷史必須拒絕，不能改寫其他專案的 receipt 或停用 trigger。
- task streams／ingress／Control product 等已實作的固定資料閉包一起刪除。未知表、尚未支援的資料種類及跨範圍引用會列入 blockers；不可把 blocker 當成已清除。預覽的 counts 是實際範圍，並非所有未來擴充功能的涵蓋承諾。
- SQLite：只接受既有精確 schema profile；工作、事件、內部關係、登記、觀察及其附表在交易中清除。永久保留的 installation receipt／decision 或其他保留資料引用目標時拒絕，保留既有不可刪除保護。其他專案及所有保留資料的完整內容摘要必須不變。
- 檔案：只接受權威清單中的絕對路徑；拒絕使用者家目錄、磁碟根、工具自身、其他專案、重疊根、祖先 junction、未支援的巢狀 repository 等。junction／symlink 僅移除連結，不追蹤目標。檔案預覽有數量、深度、manifest 大小上限。
- 這不是磁碟安全抹除：SQLite／PostgreSQL 的備份、WAL、儲存媒體殘留及外部副本不由本入口保證消失。它驗證的是支援範圍的邏輯資料與路徑不再存在。

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
  "nativeBinary": "C:/maintenance/lattice-project-purge.exe",
  "databasePath": "C:/maintenance/control.db",
  "statePath": "C:/maintenance/purge-progress.json",
  "protectedRoots": ["C:/another-project"]
}
```

```powershell
node apps/lattice-control/src/project-purge-client.mjs preview --input config.json --plan purge-plan.json
node apps/lattice-control/src/project-purge-client.mjs apply --plan purge-plan.json --confirm <預覽的 digest> --maintenance-offline
node apps/lattice-control/src/project-purge-client.mjs status --plan purge-plan.json
```

`preview` 不刪除任何資料，只產生可檢閱計畫；已有計畫檔不會被覆寫。`BLOCKED` 必須先解決列出的原因並重新預覽。`apply` 必須使用完全相同的 digest，並確認 Control／專案寫入者已停止、檔案範圍保持靜止。Node 的路徑 API 不能對抗惡意並行 rename；這不是可在線上任意執行的安全保證。

## 交易、部分失敗與續跑

1. 以獨占 operation lock 防止同一進度檔同時執行；在 SQLite `BEGIN IMMEDIATE` 內核對原始摘要並保持鎖。
2. 先核對檔案 manifest、PostgreSQL scope digest／blockers，再執行 PostgreSQL 清除交易。
3. 以 operation ID 與 scope digest 讀回 PostgreSQL receipt，接著逐項移除檔案；最後才提交 SQLite 清除。
4. 三個儲存系統無法組成單一原子交易。中斷、鎖檔、權限錯誤或回覆遺失都記為 `INCOMPLETE`；不能宣稱已回復原狀。已提交的 PostgreSQL 清除不能由 SQLite rollback 撤銷。
5. 用原計畫、原 operation ID 與進度檔續跑。PostgreSQL 回覆遺失時先讀回 receipt，不再次刪除；檔案部分失敗使用先前 removal receipt。預覽以外的變動會拒絕。歷史 receipt 還要符合目前資料的 `afterDigest`；即使是保留專案後續合法變更，也須重新核對，不能只憑舊成功紀錄宣稱目前已清空。若程序在某一檔案移除後、進度落盤前崩潰，可能須人工核對而不能自動續跑。
6. 若程序異常留下 `.lock`，先確認該 operation 已無執行程序並讀回所有階段，再處理鎖檔；程式不會自動猜測 stale lock。不得刪除未知鎖或啟動第二個 writer。
7. 三階段讀回通過只回報 `SCOPED_PURGED`。`externalCleanup: NOT_VERIFIED` 明確保留外部清理待辦；只有外部逐項驗證也完成，才可向使用者說「整個專案已清空」。

計畫／進度檔本身保留路徑及清除證據；它們也是最終資料保留決策的一部分。不要將實際專案計畫、資料庫或含機密的測試輸出提交 Git。

## 驗證

```powershell
node --test apps/lattice-control/test/project-purge*.test.mjs
cargo build -p lattice-postgres-store --example project_purge_fixture
cargo build -p lattice-runtime --bin latticed --bin lattice-project-purge
pwsh -NoProfile -File scripts/test-project-purge-postgres.ps1 -PurgeBinary <lattice-project-purge.exe 絕對路徑> -SeedBinary <project_purge_fixture.exe 絕對路徑> -RuntimeBinary <latticed.exe 絕對路徑>
```

PG harness 使用新的 loopback cluster、合成專案與任務，保留 `.lattice` 下的證據，不連正式資料庫。Windows 檔案測試包含實際鎖檔及部分失敗續跑。協調層測試使用真 SQLite／檔案與受控 native adapter；真 PG fixture 的結果須另外報告，不能把 mock 當成完整部署驗收。
