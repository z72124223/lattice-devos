# TASK-021 程式碼與安全審查

## 結論

`PASS`。TASK-021 經過初審、修復、實際 PostgreSQL 17.10 驗證及第二輪獨立
re-review（重新審查）後，最終沒有剩餘可採取的 P0、P1、P2 或 P3 finding
（問題）：P0=0、P1=0、P2=0、P3=0。程式碼／安全層面沒有 local integration
（本機整合）阻擋。

本結論只涵蓋 TASK-021 的 durable Task Ledger repository（耐久工作帳本儲存庫）、
schema v3 migration（資料庫結構第三版遷移）與 Store-v2 receipt compatibility
（Store 第二版收據相容性）。它不表示 TASK-022 以後的儲存庫、MVP-1、MVP-2、
MVP-3、production database（正式資料庫）、啟用、合併、發布或部署已完成。

## 審查目標與獨立性

- Repository：
  `C:\Users\f7212\Documents\Codex\2026-07-29\files-mentioned-by-the-user-2026\outputs\lattice-devos-v2`
- Branch：`feature/v2-rust-postgres-bootstrap`
- HEAD：`06c3954cd941dc6b743e9e2a1d4f94c3658b3ff9`
- 規格邊界：SPEC-002 v23 AC-03、AC-04、AC-35；ADR-019；TASK-021；
  Task Ledger 2.1；Postgres Store 1.3；已核准 V2 amendment（第二版修訂）。
- 初審與最終 re-review 均由未參與對應修復的獨立唯讀 reviewer（審查者）執行；
  審查者讀取規格、票券、模組憲章、Rust、SQL、測試及實際驗證結果，沒有用
  implementer self-approval（實作者自我核准）取代獨立審查。
- 目前是共享且未提交的髒工作樹，TASK-008 至 TASK-021 大量新增檔案沒有可用的
  per-ticket merge-base diff（逐票券合併基準差異）。審查主體因此由 TASK-021
  allowlist（允許清單）、當前檔案內容、下列 SHA-256 與實際測試證據固定。這是
  Git 可追溯性限制，不是產品行為缺陷。

## 初審 Findings

第一次獨立 code/security review 結論為 `BLOCK`：P0=0、P1=4、P2=2、P3=0。

| Priority | Finding | 受影響情境與違反的邊界 | 最終修復證據 |
|---|---|---|---|
| P1 | Global manifest 只在 adapter constructor（配接器建構時）驗一次 | adapter 建構後若 compatibility/history manifest 漂移，既存 instance 仍可能讀寫，違反 exact-target 與 fail-closed（失敗即拒絕） | `live.rs:598-614`、`task_ledger.rs:543-564`；Store/Ledger 每次讀寫都把 SQL 回傳的 global schema/full manifest 與建構時凍結證據比對 |
| P1 | 所有 commit database error（提交資料庫錯誤）都被當成 outcome unknown（結果未知） | 已收到明確 SQLSTATE 的 rollback／timeout 也會錯誤 poison（封鎖）instance，破壞 bounded retry 與可判定 terminal failure（終止失敗）語意 | `live.rs:844-908`、`task_ledger.rs:1734-1762`；只有沒有 DB response 才是 unknown，明確 SQLSTATE 分成 retryable 或 terminal |
| P1 | Outbox replay 未精確驗證 linkage（聯結）欄位 | 被置換的 `event_digest`、`command_id` 或 `request_digest` 可能仍經由 sequence 被讀回，違反完整 checkpoint／corruption fail-closed | `0004_task_ledger_repository.sql:1685-1693`；LEFT JOIN 同時核對 stream、sequence、event digest、command ID、request digest，計數與 Rust replay 再驗完整集合 |
| P1 | Ledger finalizer 可接受較早交易留下的 Store-only terminal | 延遲 backfill（補寫）可能把 prior transaction（先前交易）的 physical receipt 與本交易 Ledger rows 配對，違反同一交易的 atomic pair（原子配對） | `0004_task_ledger_repository.sql:2309-2345`；要求 terminal `xmin = pg_current_xact_id()::xid`，只接受當前外層交易剛建立的 terminal |
| P2 | Load 未拒絕 wrong-scope physical collision（錯誤範圍實體碰撞） | 相同 owner/aggregate 但不同 project/snapshot 的實體列可能被忽略，破壞專案／快照隔離 | 初次修復涵蓋已存在 stream；最終再審查進一步補齊 vacant stream，詳見下方第二輪 P2 |
| P2 | 所謂 bounded transaction（有界交易）沒有 lock/statement timeout | database lock 可無限等待，與票券的有界失敗要求不一致 | `live.rs:56-67`、`task_ledger.rs:150-161` 及 `0004` 八個函式 `proconfig`；固定 `lock_timeout=5s`、`statement_timeout=30s`，`55P03`/`57014` 映射 `Unavailable` |

初審問題未被風險接受或降級；六項都先修正、加回歸證據，再進入 re-review。

## 修復後實機驗證發現與修正

初審修復後第一次完整 PostgreSQL initial phase（初始階段）沒有被誤列為 PASS：
同 command／同 stream 的第一個 live 情境回傳 `RetainedRowCorrupt`。加入可追蹤的
靜態 stage marker（階段標記）後，focused proof（聚焦證明）定位出精確根因：

- SQL Ledger finalizer 無條件要求 fresh Store genesis state `G` 等於 vacant Ledger
  checkpoint `B`。兩者是不同契約的 structural genesis（結構型初始值），不能假設
  digest 相等。
- 修復後，只有 `v_stream_found` 時才要求 Store terminal 的 expected/before state
  等於既存 Ledger base checkpoint；fresh stream 仍必須是 structural-zero、physical
  revision 0，並只允許 0 -> 1，after state 必須等於 next Ledger checkpoint。
  最終條件見 `0004_task_ledger_repository.sql:2274-2345`。
- initial phase 修正後通過；restart phase（重啟階段）又揭露測試 fixture（夾具）
  結束時仍留下 `ACTIVE` admission，導致重啟 schema verifier 正確拒絕。測試收尾改為
  `STOPPED`／no-leader 後，initial 與 restart 均通過；這是測試生命週期修正，沒有
  放寬 production verifier。

這兩次失敗均保留 fail-closed 行為，沒有以刪除 assertion（斷言）或放寬驗證規則
換取綠燈。

## 第二輪 Re-review Finding

完整 initial/restart 通過後，最終獨立 code re-review 仍找到一個新的 P2：

- **P2 — vacant load 仍可能忽略 wrong-scope physical orphan（錯誤範圍實體孤兒列）。**
  若沒有 `task_ledger_streams` row，但相同 `TASK_LEDGER` owner + stream ID 已在另一個
  project/snapshot 留下單一 `physical_heads` row，舊 read-head 路徑會回傳正常 vacant。
  這違反專案／快照隔離與 corruption fail-closed。
- 修復把 `task_ledger_read_head_v1` signature 固定為 `(bytea,text,text)`，Rust load
  傳入預期 project/snapshot；SQL 即使 stream 不存在也會全域檢查相同
  owner/aggregate 的 wrong-scope 或 duplicate physical rows，命中時以 `LCR01` 拒絕。
  證據位於 `task_ledger.rs:52-64,543-564` 與
  `0004_task_ledger_repository.sql:1368-1542`。
- 直接 PostgreSQL regression（回歸測試）建立 vacant wrong-scope orphan，驗證讀取
  fail closed 且 mutation-count vector（異動計數向量）保持 `[1,0,0,0,0,0]`；測試位於
  `postgres_live.rs:2318-2390`。修復後完整 initial/restart harness 再次 PASS。

第二輪修復的 changed hunks（變更區塊）再經獨立審查，最終 P0=0、P1=0、P2=0、P3=0。

## 最終正確性與安全結果

- Task Ledger 2.1 保持 pure/zero-I/O（純函式／零輸出入），Fake 與 Live 共用同一
  vacant/plan/apply/checkpoint 邊界；既有 request/event/head/receipt hashes 不變。
- `PostgresTaskLedger` 只接收 caller-supplied authenticated `Client` 與 exact
  `MigrationTarget`，沒有 raw client getter、任意 SQL、DSN、password、credential 或
  environment discovery（環境探索）出口。
- 新 command 在同一個 `SERIALIZABLE` 交易內先分類 exact retry／changed reuse，再
  重驗 ACTIVE daemon authority 與 physical head，依序呼叫 `store_finalize_v3` 和
  `task_ledger_finalize_v1`，兩者成功後才 commit；任一步失敗會回滾全部。
- Exact retry 可跨 later event、STOPPED、epoch change、restart 或 commit-response
  loss 重建同一 Ledger/Store receipt；changed reuse 不洩漏 retained receipt。
- Commit 明確 SQLSTATE 只依 allowlist 做最多三次 pre-commit retry；unknown response
  不回傳 receipt、poison 舊 instance，需新 client 加 exact replay 對帳。
- 完整 Ledger checkpoint 綁定 identity、head、resource projection、ordered events、
  所有 appended/denied commands 與 outbox admissions。stale/overflow denial 仍寫一個
  terminal command 並前進 checkpoint，但不建立 event/outbox。
- 只有 appended `EFFECT_INTENT` + audit outcome `RECORDED` 建立一個 immutable
  `ADMITTED` outbox row；event/outbox 的 digest、command、request linkage 全部重驗。
- Ledger finalizer 只接受本交易 `xmin` 的 Store terminal；Store-only partial pair
  不會被靜默補齊，任一不一致都 fail closed。
- 每個 runtime function 都是固定 signature、migrator-owned `SECURITY DEFINER`、
  schema-qualified、無 dynamic SQL、`search_path=pg_catalog`、`row_security=on`、
  `lock_timeout=5s`、`statement_timeout=30s`。
- Runtime 只有三個 Store-v3 與五個 Ledger-v1 function `EXECUTE`；historical
  Store-v2 functions 的 runtime grant 已撤銷。`PUBLIC`、Guardian、reader、pre-SET
  ROLE login 均不可執行，runtime 對六個 protected tables 維持零直接 SELECT/DML。
- 沒有發現 SQL injection（SQL 注入）、secret retention（祕密保留）、第二個 durable
  truth、第二個 product writer、跨 project/snapshot/stream 讀寫、daemon self-activation
  或 companion/playmate／陪玩網站耦合。

## 最終驗證證據

主工作流在最終凍結快照實際執行並通過：

- `cargo test -p lattice-task-ledger --locked`：25/25（12 unit + 13 integration）。
- `cargo test -p lattice-postgres-store --all-targets --locked`：所有 focused/static
  tests 通過；migration contract 15/15。一般 env-gated marker test 在沒有 harness
  environment 時按設計不冒充 live PostgreSQL 證據。
- `powershell -File .\scripts\run-task019-postgres.ps1`：
  `TASK019_HARNESS_SELF_TEST=PASS`、`TASK019_POSTGRES_HARNESS=PASS`；PostgreSQL 17.10
  marker-owned、non-5432 loopback cluster 的 initial 與 restart phases 均 PASS。
- Live matrix 直接涵蓋 fresh/v1/v2/v3 migration、historical Store-v2 replay、
  vacant/load/append、exact/changed/stale/overflow、conditional outbox、same-command/
  same-stream/cross-stream concurrency、Store->Ledger rollback、serialization exhaustion、
  commit-response loss/poison/reconnect、manifest drift、lock timeout、outbox linkage、
  existing/vacant wrong-scope corruption、ACL、restart，以及 PostgreSQL 17 `xmin`
  same-/cross-transaction primitive（同／跨交易原語）。
- `cargo test --workspace --all-targets --all-features --locked`：432/432 Rust tests。
- `npm.cmd run verify`：44/44 Node tests，governance check 通過。
- `cargo fmt --all -- --check`：exit 0。
- `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`：exit 0，
  zero warnings。
- `cargo audit --file Cargo.lock`：109 dependencies，zero known vulnerability。
- Dependency tree（依賴樹）只新增核准的單向
  `lattice-postgres-store -> lattice-task-ledger` 與 exact serde-json support；沒有 cycle
  或 adapter-to-adapter dependency。
- `git diff --check`、conflict-marker、scope、secret/DSN/raw-client、dynamic-SQL、
  migration/table/function/ACL scans：通過。

測試 PASS 被當作行為證據，而不是取代規格、所有權、安全及失敗路徑的獨立審查。

## Frozen Evidence

Migration manifest（遷移清單）的最終凍結值：

| File / evidence | Bytes | SHA-256 |
|---|---:|---|
| `0001_bootstrap.sql` | 312 | `7bff021fc17f738551309c906578c8015b2dd0307d27d239c21df1697c4d09c8` |
| `0002_control_store_foundation.sql` | 14,259 | `e996dc64af3112a647e75ebf07df2a77b1e9b3a018ed443880150365184883f0` |
| `0003_live_control_store.sql` | 29,518 | `00ae3eedd76704f26b1df58955d9d594c98f0ba525be93b15d8c9ebb1f2115c1` |
| `0004_task_ledger_repository.sql` | 111,742 | `cd658ed2f4624cd0a829c818c1cf96d8ac3829264046976bdde3b2fc7feea6e5` |
| Full four-entry manifest | — | `09c431df18ad71a4f44239a5d2ddf6b1774b8ffec06c7f9223f0e41757f3d407` |
| Frozen Store-v2 first-three-entry manifest | — | `4582edce68a947998a8f4c6895bb37ceec9e842f516471f4d9e2617a6757f129` |

早期修復中的 `0004`／full-manifest 暫存雜湊已被 fresh-genesis 與 final wrong-scope
修正取代，不是交付證據；上表是唯一最終 freeze（凍結值）。

V3 live catalog signatures（實際目錄簽章）：

- relation：`85619233866577a32550fac8f83f9995c05f24ddeaaf64d9563609e6c9ac8767`
- column：`7cd1aa5142dbccdc2ac2db466ba4ffdf0c9c41a1000ae0c59baf650e36bbaae8`
- constraint：`f9d587125d792646b77ca68e6224c9866bd32c87a0e98c4d2f85b75dd0c22be8`
- index：`40ca5ea0781b1be03efe9bead50ae9f78434314123d6f700d278874678d06a9b`
- function：`f2c8585e1da944b38a50c65c6b9f448963f4c3d96c909331be87fec0c30d2279`
- function ACL：`579b843df8e187eb0f4b7a75e9d1b0c4f109d596c55bcff5aa76a1a06bfcd91b`
- table ACL：`27a0879d1b709abd341653b445d3a64d59819bde2e20e868ac09d2624aab1993`

最終主要實作／測試檔案 SHA-256：

- `crates/lattice-task-ledger/src/lib.rs`：
  `866a9ff25f8ee91a35675d9d419c8181d9bbc8240f96bf8c39ab7239587561d6`
- `crates/lattice-task-ledger/tests/task_ledger.rs`：
  `9ffb13e308984f75e9e53ef991e69306647a7d95bcf7e9d13426767b6649f393`
- `crates/lattice-postgres-store/src/live.rs`：
  `6261f04dc23fc5c00d5200eef6d5c7eb576527dc0fedb82592a821013831f842`
- `crates/lattice-postgres-store/src/migrations.rs`：
  `b1ef8b4db7c8d853668041a8e6db30b14f54e01c92e29f9e1278cac471cade57`
- `crates/lattice-postgres-store/src/postgres_setup.rs`：
  `102e2cc5e6a9505d912b0159f3a4525728fb1a415a69d12bded9b0dc2d7b5ff8`
- `crates/lattice-postgres-store/src/task_ledger.rs`：
  `696d0a145d3cf22f227659b96f51369930965f7055b95fdba3423bcba2f82c7d`
- `crates/lattice-postgres-store/tests/migration_contract.rs`：
  `7a93613e247e2ed0249fb5c8698ab86e53e8b0af765bafe0ed94dc529dbe6a22`
- `crates/lattice-postgres-store/tests/postgres_live.rs`：
  `8b8ff37ca7650e6716b57eb1eed5058cbd5e6c55a85abb27b4f285df6133e46a`
- `scripts/run-task019-postgres.ps1`：
  `cf9f057b23d5de83d37c1101ff29414b67b138a57c7857209492c34a8abc8ecf`

## 殘餘風險與證據邊界

- 70-argument Ledger finalizer 的 delayed-backfill 情境由 SQL static contract
  （靜態契約）核對，加上 PostgreSQL 17 `xmin` same-/cross-transaction live primitive
  證明；未另建脆弱且重複綁死 70 個 raw scalar arguments 的直接呼叫測試。這是已知
  evidence-shape risk（證據形狀風險），不是未解決 correctness finding。
- 一般 workspace test 不會自行啟動 PostgreSQL；live correctness 必須持續由
  marker-owned harness 證明。本次已有實際 17.10 initial/restart PASS，但沒有建立
  remote/TLS 或 production compatibility claim（正式相容性主張）。
- 完整 per-append replay 對單一 stream 歷史目前可能為線性成本；snapshotting、
  pagination 與效能界線需要後續量測與版本化決策，不能用來放寬目前 correctness。
- Remote CI、branch protection、required remote review、upstream synchronization、
  committed candidate 與 primary-branch merge authorization 都是 missing/unverified。
- Live resource observation、outbox claim/delivery/reconciliation、其他 domain
  repositories、Writer Lease composition、provider/product、Guardian、正式憑證／資料庫、
  release、deploy、commit、push 或 merge 均未由 TASK-021 執行或驗證。

## Final Blocker Status

最終程式碼與安全審查：`PASS`，P0=0、P1=0、P2=0、P3=0。
TASK-021 可交給 architecture review（架構審查）及 local integration；不可從本報告
推論 primary branch merge、production migration、activation、release 或 deployment
已獲授權或完成。
