# Windows 共用服務的手動啟動與停止

封存 Codex 對話不代表要求停止共用 Control 或 PostgreSQL。以下入口將啟動命令
正常結束，保留手動啟動的背景程序；不新增排程、自動啟動、重新啟動或監督器，
不修改 Windows Job、Codex 或工具政策。需要 PowerShell 7.5、Node 24.15+、PostgreSQL 17。

每次啟動使用全新的狀態檔，父目錄須先存在。不可直接接管未由該次啟動產生的程序。
下列絕對路徑由執行端依已核對的安裝與資料位置填入；不要把範例資料庫當作正式庫。

```powershell
pwsh -NoProfile -File scripts/Start-LatticeSharedControl.ps1 -NodePath <node.exe絕對路徑> -DatabasePath <Control資料庫絕對路徑> -StatePath <新control.json絕對路徑>
node apps/lattice-control/src/shared-control.mjs status --state <control.json絕對路徑>
node apps/lattice-control/src/shared-control.mjs stop --state <control.json絕對路徑>

pwsh -NoProfile -File scripts/LatticeSharedPostgres.ps1 -Action Start -ClusterPath <既有cluster絕對路徑> -StatePath <新postgres.json絕對路徑>
pwsh -NoProfile -File scripts/LatticeSharedPostgres.ps1 -Action Status -StatePath <postgres.json絕對路徑>
pwsh -NoProfile -File scripts/LatticeSharedPostgres.ps1 -Action Stop -StatePath <postgres.json絕對路徑>
```

Control 停止前重核 PID、建立時間、完整執行檔路徑與目前 Windows 帳戶。
管理通道僅監聽 loopback，每個連線使用一次隨機 challenge 與雙向 HMAC。
狀態檔中的隨機金鑰以 DPAPI CurrentUser 保護並綁定實例及路徑，明文不寫入
參數、環境變數或日誌。同帳戶惡意程式不屬於此保護邊界。
正常停止呼叫既有 `shutdownOwned`；`SHUTDOWN_INCOMPLETE` 不代表已停止，
再次請求也不保證能修復應用程式已保存的失敗。簽署的 STOPPED 收據須搭配 OS
程序退出讀回，才構成完整停止驗收。

PostgreSQL 額外重核 cluster 的 system identifier、`postmaster.pid` 與命令列的
精確 `-D` 路徑。`pg_ctl` 啟動器最多等待 35 秒；逾時記為
`START_OUTCOME_UNKNOWN`，不得自動再啟動或宣稱資料庫未啟動。
停止僅使用 `pg_ctl -m smart -w -t 30 stop`，有未結束連線時可逾時，
不升級成 fast、immediate 或按程序名稱終止。先查原啟動紀錄及該 cluster 狀態。

## 驗收

```powershell
node --test apps/lattice-control/test/shared-control.test.mjs
# RunId 是每次測試新產生的 32 字元小寫十六進位值。
pwsh -NoProfile -File scripts/test-shared-services.ps1 -Action Start -RunId <新RunId> -NodePath <node.exe絕對路徑>
pwsh -NoProfile -File scripts/test-shared-services.ps1 -Action Verify -RunId <同一RunId> -NodePath <node.exe絕對路徑>
pwsh -NoProfile -File scripts/test-shared-services.ps1 -Action Stop -RunId <同一RunId> -NodePath <node.exe絕對路徑>
```

測試僅建立 LocalAppData/LATTICE/shared-service-tests 下的獨立空 Control 與
合成 PostgreSQL 資料，不連正式 runtime。Verify 核對同一程序實例及持久資料，
Stop 核對正常停止及 OS 退出；原始輸出留在該 RunId 目錄。

要宣稱通過「封存後仍存活」，還必須在使用者允許建立的專用 Codex 測試對話中
執行 Start，等該命令及回合結束，再用 Codex 原生工具封存該對話，從另一對話
執行 Verify 並讀回同一 LATTICE 工作身分。只斷開 socket、指令退出、模擬封存
或重啟服務都不能取代此驗收。測試工具本身固定輸出 `nativeArchive=NOT_ASSERTED`。
