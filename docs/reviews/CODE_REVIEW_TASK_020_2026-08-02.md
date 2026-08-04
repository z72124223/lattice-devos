# TASK-020 程式碼與安全審查

## 結論

`PASS`。對凍結後最新工作樹完成獨立 code/security review
（程式碼／安全審查），沒有剩餘可採取的 P0、P1、P2 或 P3 finding
（問題）：P0=0、P1=0、P2=0、P3=0。TASK-020 沒有程式碼或安全整合阻擋，
可交給 architecture review（架構審查）與 local integration（本機整合）繼續。

## 審查目標與獨立性

- Repository：
  `C:\Users\f7212\Documents\Codex\2026-07-29\files-mentioned-by-the-user-2026\outputs\lattice-devos-v2`
- Branch：`feature/v2-rust-postgres-bootstrap`
- HEAD：`06c3954cd941dc6b743e9e2a1d4f94c3658b3ff9`
- 規格邊界：SPEC-002 v22 AC-34、ADR-018、TASK-020、Contracts 1.9、
  Ports 1.4、Postgres Store 1.2。
- 審查者未參與 TASK-020 產品程式碼實作；本次只讀取規格、模組契約、Rust、
  SQL、harness（測試驅動器）與目前驗證證據。唯一寫入是本報告。
- 這是共享且未提交的髒工作樹，無法從 merge-base（合併基準）產生純
  TASK-020 Git diff；因此以 ticket allowlist、TASK-019 已凍結基線、目前檔案內容
  與下列 SHA-256 固定審查主體。這是可追溯性限制，不是產品缺陷。

## Findings

`No findings`。

| Priority | Open count | Integration effect |
|---|---:|---|
| P0 | 0 | none |
| P1 | 0 | none |
| P2 | 0 | none; no risk acceptance required |
| P3 | 0 | none |

## 規格、正確性與安全結果

- Contracts 1.9 保留 v1 fake-only（僅假實作）相容性；v2 只允許精確的
  Fake/NonDurableFake/no-persistence 或 Live/DurablePostgres/complete-persistence
  組合。live daemon ID 的 SQL 可表示前綴限制只套用到 Live，未破壞 v1 fake。
- Ports 1.4 只把 `ControlStore::current_head` 改成明確的 `&mut self`；Ports 仍只依賴
  Contracts，沒有 driver（驅動程式）、SQL 或 connection（連線）型別外洩。
- `PostgresControlStore::new` 只消耗 caller-supplied（呼叫端提供）的 runtime
  `Client` 與精確 `MigrationTarget`，先完成 schema/identity/role/ACL 驗證；公開 API
  沒有 raw client、任意 query、DSN、password、credential source 或 environment
  內容出口。
- 每次新交易在一個 `SERIALIZABLE`（可序列化隔離）交易內完成 prepare、Rust
  canonical hash（標準化雜湊）、finalize 與 commit。只有 commit 成功後才回傳 durable
  receipt；非已知可安全重試的 commit error 會回傳 `CommitOutcomeUnknown` 並 poison
  該 Store instance（要求更換連線後精確重放）。
- SQL 在 transaction-ID advisory lock（交易識別建議鎖）下先做 exact replay／
  changed-ID 分類，再檢查 mutable admission（可變准入）、daemon authority 與 locked
  physical head；finalize 會重新呼叫 prepare 並重驗 authority、head、revision、
  disposition 與 persistence evidence。
- APPLIED 只前進一個 checked signed-BIGINT revision；STALE 會留下 terminal receipt
  而不異動或物化 first-use genesis。`0003` 移除 v1 terminal-to-head FK，且整份 SQL
  只有 APPLIED 分支的一個 `INSERT INTO control.physical_heads`。
- 重放會從 retained row 重建 canonical before/after/transaction/receipt digests，並把
  database identity、schema version 與 manifest commitment 與目前 verified target
  比對；NULL、shape drift（形狀漂移）、digest substitution 與非 canonical head 均
  fail closed（封閉式失敗）。
- 三個 `SECURITY DEFINER`（以擁有者權限執行）函式固定 signature、owner、source、
  properties、`search_path=pg_catalog`、`row_security=on` 與 ACL；只允許
  `lattice_runtime` EXECUTE。`PUBLIC`、Guardian、reader 與所有 pre-`SET ROLE` LOGIN
  無權執行，runtime 對 physical/terminal tables 仍無直接 SELECT/DML。
- v1-to-v2 runner 只接受 fresh、完整精確且空的 v1 prefix、或完整精確 v2；v1
  catalog、history、compatibility、identity、STOPPED admission、roles/ACLs 與空表在
  migration 前鎖定並驗證。history 與 compatibility 在同一 runner transaction 內更新。
- 未發現 SQL injection（SQL 注入）、dynamic SQL（動態 SQL）、secret retention
  （祕密保留）、跨 scope 讀寫、第二 durable truth、第二 product-code writer、
  runtime self-activation 或 companion/playmate 專案耦合。

## 已修舊 Finding 的再審查

下列較早 review finding 已在凍結快照中修正並重新核對：

1. Commit error 分類：`live.rs` 現在只對 SQLSTATE `40001`、`40P01`、以及固定函式
   first-row race 的 `23505` 做 bounded whole-attempt retry；其他 commit response error
   一律為 unknown outcome，且 Store instance 進入 reconciliation-required。
2. Daemon identifier 對齊：Live authority 要求首 byte 為 ASCII alphanumeric，與
   `0003` regex 一致；既有 fake v1 punctuation-first identifier 仍可建構。
3. Retained nullable 欄位：replay/substitution 比對改用 `IS DISTINCT FROM`，避免
   NULL 三值邏輯讓 checkpoint/outbox 或受損欄位繞過比較。
4. Idempotency 順序：terminal lookup 與 changed-ID classification 位於 mutable
   admission 前；changed admission 的同 ID 請求回 `CommandSubstitution`，exact retry
   在目前 admission/epoch/head 已改變後仍可重放。
5. First-use stale：移除 v1 scope-head FK，prepare 只使用 virtual genesis；STALE
   不再偷偷建立 physical head。
6. 文件語意漂移：Postgres Store constitution 的舊 fake-only durability Non-Goal、
   Ports trait 註解及 Store crate 頂層說明已改成與 ADR-018/live durable 實作一致；
   focused format/Clippy/tests 再次通過，沒有行為變更。

## 測試品質與驗證證據

獨立審查實際執行並通過：

- `cargo test -p lattice-contracts --test store_contracts --locked`：10/10。
- `cargo test -p lattice-ports --test store_port --locked`：2/2。
- `cargo test -p lattice-postgres-store --locked`：所有非環境依賴測試通過；其中
  `marker_owned_postgres_17_foundation` 在未設定 live harness environment 時按設計 no-op，
  不被誤列為實際 PostgreSQL 證據。
- `cargo test --workspace --all-targets --all-features --locked`：409/409。
- `npm.cmd run verify`：`check=ok files=254 constitutions=18 tickets=20
  current_tasks=1`，Node 44/44。
- `cargo fmt --all -- --check` 與針對 Contracts/Ports/Postgres Store 的 strict
  `cargo clippy ... -- -D warnings`：exit 0。
- `cargo tree -p lattice-postgres-store --edges normal --locked`：只有 Contracts、Ports、
  cjson、精確 `postgres = 0.19.14`、`sha2 = 0.11.0` 與其正常 transitive dependencies；
  沒有 domain/provider/product edge。
- `cargo audit`：掃描 `Cargo.lock` 109 dependencies，exit 0、零已知 vulnerability。
- `git diff --check`、conflict-marker scan、migration hash/length scan、raw-client/secret/
  DSN/dynamic-SQL scan：通過。

凍結工作樹的交付證據另含一個實際 marker-owned PostgreSQL 17.10 harness run：

- initial 與 restart phases 均為 `TASK019_POSTGRES_HARNESS=PASS`；
- 包含 fresh v2、exact/no-op、exact v1 upgrade、concurrent upgrade/runner、negative v1
  matrix、rollback、real LOGIN/ACL、apply/stale/replay/substitution、same-ID/same-scope/
  cross-scope concurrency、四次 serialization failure exhaustion、signed-BIGINT overflow、
  commit-ack loss/no-receipt/poison/reconnect replay、retained NULL/digest corruption、restart
  replay、service-preserving stop/cleanup；
- 這個 live harness 證據與一般 `cargo test` 的 env-gated no-op 明確分開。

## 凍結檔案雜湊

- `crates/lattice-contracts/src/lib.rs`:
  `973016cbfc25ef4d08aa27b7ea6e93b3930067215713519340aae5eb6ddf6828`
- `crates/lattice-ports/src/lib.rs`:
  `5ec4e1afd49fe31d5e5d0d15632f7351aaa3385f8f6c694f1e0be6a699c851f7`
- `crates/lattice-postgres-store/src/live.rs`:
  `fd736fc3f4d37a4afc189b3eda3f9a34eed1999b18bcc74388499d8932f2f400`
- `crates/lattice-postgres-store/src/migrations.rs`:
  `18619669c3cd8646a2e13912f528266bb8e2770f9d9ad30eb6198b96e8bb24be`
- `crates/lattice-postgres-store/src/postgres_setup.rs`:
  `d163cdf52e30768615868ec295c69f42dad5b42391f7b9deceaff8db34d6454c`
- `crates/lattice-postgres-store/tests/postgres_live.rs`:
  `44c0a25649f749d25c9a6e0e89df4be450c21c390e8cf030ed1c73648a2706bc`
- `db/migrations/0003_live_control_store.sql`:
  `00ae3eedd76704f26b1df58955d9d594c98f0ba525be93b15d8c9ebb1f2115c1`

Preserved migration evidence：

- `0001_bootstrap.sql`: 312 bytes,
  `7bff021fc17f738551309c906578c8015b2dd0307d27d239c21df1697c4d09c8`
- `0002_control_store_foundation.sql`: 14,259 bytes,
  `e996dc64af3112a647e75ebf07df2a77b1e9b3a018ed443880150365184883f0`
- `0003_live_control_store.sql`: 29,518 bytes，雜湊同上。

## 殘餘風險與範圍

- 一般 workspace test 不會自行啟動 PostgreSQL；live correctness 必須持續以獨立
  marker-owned harness 證明。本次有實際 17.10 initial/restart PASS，但未建立 production
  相容性主張。
- 目前沒有 remote CI（遠端持續整合）、branch protection、required remote review、
  committed candidate 或 upstream。這些是 missing/unverified enforcement，不影響可逆
  本機 TASK-020 結論，但阻擋 merge-readiness（可合併狀態）主張。
- 工作樹包含 MVP-0 至 TASK-020 的共享未提交變更；local integration 必須以本報告雜湊
  與 ticket allowlist 固定主體，不能宣稱純 per-ticket Git diff。
- Production database/login/credential、remote/TLS、real daemon/Guardian activation、
  domain repositories、outbox/filesystem effects、provider/product、release/deploy/merge
  均不在 TASK-020，沒有執行或驗證。

## Final Blocker Status

程式碼與安全審查：`PASS`，P0=0、P1=0、P2=0、P3=0。
TASK-020 可進入 architecture review 與 local integration；不可由本報告推論 primary
branch merge、production migration、activation、release 或 deployment 已獲授權或完成。
