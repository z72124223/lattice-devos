# 備份恢復的隔離界線

`lattice-customer-backup.py restore` 把已驗證的封存還原至新的獨立目錄；
`finalize-restore` 核對安裝、catalog 與保存的專案 proof，回報
`CUSTOMER_RESTORE_QUARANTINED`。兩個入口都不啟動 PostgreSQL／Runtime，
不 bootstrap schema，不呼叫 Registry restore 啟用專案，也不寫入 `ready.json`。

`restore.pending.dpapi` 與 `restore.in-progress` 保留，包括重試與報告寫入中斷後。
即使舊恢復紀錄含成功啟用結果或已有 `ready.json`，這些紀錄也不授予本次再啟用權限。
本候選沒有自動解除或手動解除隔離的產品入口；不要直接移除閘門來完成恢復。

最新的停用／保留決策由現用環境及使用者當前決策保管，不能從舊備份推定。
未來重新啟用前，需要目前的 owner 明確核對哪些專案仍可啟用，以及 Registry、
Control、Bot、Graphify、排程與對話範圍。混合備份完整保留，不因某專案停用而整份刪除。
本候選預設整份恢復副本隔離，沒有新增可由備份自行填寫的可信清單。

現有 customer manager 的 `load` 在讀取憑證前檢查上述閘門；正常 `serve` MCP、
start／recover、connect、Graphify、登記專案及匯入結果入口都經過它。
Codex 設定指向同一 `serve` launcher，因此此入口不會啟動其下游 Runtime／工作。
這不是 OS 隔離、資料庫 admission 改寫或全系統撤銷：不會停止另外啟動的程序，
也不能防止同一 OS 使用者直接操作複製出的資料庫或原生執行檔。

隔離報告的 `database_state: NOT_CHECKED` 表示未查驗或改寫備份內的 active 狀態；
`project_activation: NOT_PERFORMED` 僅描述這次恢復入口沒有執行啟用。
`CURRENT_OWNER_REVIEW_REQUIRED` 不是恢復後正式服務可用的驗收結果。
Registry 外部 epoch anchor 僅防資料庫單邊的跨 epoch 回滾；資料庫內 tombstone、
舊備份的 catalog 或舊 approval 都不能證明目前仍有再啟用授權。

聚焦測試：以既有 Python 執行 `-I -B -S scripts/test_lattice_customer_backup.py`。
測試用合成檔案和受控 adapter；不代表真 PostgreSQL、完整備份還原或正式部署驗收。

`scripts/test-customer-restore-quarantine-postgres.py` 是額外的 Windows PG17
隔離驗證入口，需明確指定 runtime 與 fixture seed binary。它只在本 checkout
的 `.lattice/customer-restore-quarantine-fixture/<隨機 ID>` 建立合成 loopback 庫，
保留所有證據並在結束時依 marker、PID、程序身分停止自身實例。
驗證目標是停庫實體複製、DPAPI journal、實際 finalizer 重試及 manager 啟動閘，
不涵蓋 AES 備份解密、WSL 或整套客戶恢復流程；只有該次 `result.json` 的 PASS
才能證明其列出的檢查通過。
