# 產品計畫

## 權威與交付

- Runtime 工作真相由 LATTICE／PostgreSQL 保存；Codex task 是推理與施工現場，
  不取代 durable task、attempt、receipt 或 execution-environment authority。
- 程式交付真相由 GitHub 提交、PR 與 CI 保存。
- 正式產品分支是 `product/lattice-control-mvp`。
- 易變的執行結果、PR、CI 與安裝觀察不複製到本檔；歷史保留在 Git 與 LATTICE。
- 根目錄文件只描述穩定產品方向，不充當工程看板或施工日誌。

## 已交付基線

- 本機優先的 LATTICE Control，可保存專案、工作項目、Codex task 關聯、
  進度、核准、驗證與封存狀態。
- PostgreSQL Task Ledger、writer lease、Project Registry、managed Foreman、
  Artifact Store 與可重播的 runtime observation。
- 一般任務以 create-only ingress 進入既有 Task Ledger，不建立 Control
  影子任務或第二份狀態機。
- 工頭可用封閉的 dependency binding 建立子工作，只有 live Git 證據能解除
  阻塞；衝突、漂移或不確定狀態一律 fail closed。
- Codex App Server 的 connect、turn readiness、interrupt、bounded retry、
  resume 與 reconcile 都以精確 thread／turn identity 關聯。
- 受管執行環境固定 toolchain、credential boundary、process fence、Git
  worktree 與 output bounds，並以獨立 verifier 產生有界證據。
- Runtime、Control 與本機 project catalog 保持既有 API／CLI／持久層；
  catalog observation 不冒充更高層 authority。

## 下一個產品里程碑

1. 維持 Store、Writer、Foreman 與 Runtime 的精確 catalog／ACL 相容性，
   所有 extension 只允許經驗證的版本化物件。
2. 讓 Control 的 radar、project catalog 與安裝 observation 在重新啟動後
   可精確重播，且持續標示 authority 邊界。
3. 將每次候選交付綁定同一 Git commit、release artifact、installed smoke、
   PR checks 與產品分支 ancestry。
4. 對高風險執行保留一次有界 live attempt、獨立 review、明確 rollback 與
   fail-closed cleanup。

## 穩定非目標

- 不新增第二套 task truth、agent loop、shadow scheduler 或平行 deployment
  workflow。
- 不把本機 Control catalog 當成 PostgreSQL、LATTICE 或 GitHub authority。
- 不在沒有個別授權時執行付款、公開發布、帳戶／credential 變更、永久刪除、
  primary-branch merge、deployment 或 release。
- 不以測試 fixture、focused smoke、舊 receipt 或記憶摘要冒充 installed
  product 的目前狀態。

## 產品擁有者決策

- 專案尚未選定開源授權；公開可見性與 clone 能力不等於開源授權。
- 新增公開雲端服務、付費依賴或對外資料處理前，需另行確認產品與權限邊界。
