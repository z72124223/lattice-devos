# TASK-021 Architecture Review

## Decision

`PASS`。最終獨立 architecture re-review（架構重新審查）沒有剩餘 P0、P1、P2
或 P3 finding（問題）：P0=0、P1=0、P2=0、P3=0。TASK-021 沒有 architecture
integration blocker（架構整合阻擋），也不需要新的 ADR 或 module constitution
amendment（模組憲章修訂）。

本決策只接受第一個 durable Task Ledger repository（耐久工作帳本儲存庫）及其
PostgreSQL schema v3 compatibility unit（相容單元）。它不把 TASK-021 擴張成完整
MVP-1、One Writer 組合、effect delivery（效果送達）、MVP-2 元件整合、MVP-3
Guardian、正式環境、發布或部署完成。

## Review Target And Independence

- 規格與決策：SPEC-002 v23 AC-03/04/35、ADR-019、approved V2 amendment、
  TASK-021。
- 模組契約：Task Ledger 2.1、Postgres Store 1.3；參照 Contracts、Ports 及
  TASK-020 Store-v2 baseline（基線）。
- 實作主體：Task Ledger pure planner/replay、`PostgresTaskLedger`、manifest runner/
  verifier、`0004_task_ledger_repository.sql`、live PostgreSQL harness。
- 最終 reviewer 未參與相應 code repairs（程式修復），以唯讀方式重新檢查所有權、
  契約、依賴、失敗模式、migration、ACL、實機證據及初審 finding resolution。
- 共享未提交工作樹無法提供純 TASK-021 merge-base diff；審查主體由 ticket
  allowlist、current files、final hashes 與驗證證據固定。沒有宣稱 committed candidate
  或 remote synchronization（遠端同步）。

## Review Triggers

TASK-021 命中全部必要 architecture-review triggers（架構審查觸發條件）：

- Task Ledger public contract 從 2.0 進到 2.1，新增 runtime-aware vacant/plan/apply、
  retained commands、complete checkpoint 與 conditional outbox admission。
- Postgres Store 從 1.2 進到 1.3，新增一個 domain-planned live adapter 及單向依賴。
- Global PostgreSQL schema 從 v2 進到 v3，新增四表、八個固定函式、ACL 與 migration
  compatibility path（相容遷移路徑）。
- 交易 atomicity（原子性）、commit uncertainty、locking timeout、restart replay、
  corruption handling、project/snapshot isolation 與 security-definer privileges 都是
  material reliability/security boundaries（重大可靠性／安全邊界）。

## Before And After Architecture

| 面向 | TASK-020 baseline | TASK-021 最終狀態 |
|---|---|---|
| Task Ledger ownership | Pure event/request/head/receipt/resource replay 與 Fake | 仍 pure/zero-I/O；新增 shared vacant/plan/apply、retained terminal commands、complete checkpoint、conditional outbox admission |
| Physical Store | schema v2 physical `ControlStore`、Store-v2 durable receipt | schema v3 global catalog，Store-v2 receipt profile 保持不變；新增 Ledger-planned adapter |
| Durable transaction | 只原子提交 physical head/terminal | 同一外層 `SERIALIZABLE` transaction 依序 finalize Store 與 Ledger，command/event/outbox/projection/checkpoint/physical receipt 全有或全無 |
| Mutable semantic owner | 各 domain module 自己判定合法 transition | 不變；Task Ledger planner 建構語意，SQL 只鎖定、重驗與持久化 |
| Outbox | Store 只有 opaque optional intent commitment | Ledger 對 appended `EFFECT_INTENT` + `RECORDED` 唯一導出 immutable admission；未 claim/deliver |
| Dependency | Postgres Store 不依賴 domain repository | 新增唯一核准的 `postgres-store -> task-ledger`；沒有反向邊、cycle 或 adapter-to-adapter call |
| Runtime DB surface | 三個 Store-v2 function | 歷史 v2 grants 撤銷；runtime 只可執行三個 Store-v3 + 五個 Ledger-v1 fixed functions，protected tables 零直接權限 |

## Boundary And Ownership Result

- **One Gateway**：沒有新增一般使用者入口、OpenClaw、Codex、IPC 或平行
  orchestration surface（編排介面）。
- **One Truth**：PostgreSQL 仍是唯一 live durable truth；Task Ledger pure planner 和
  Fake 不聲稱 durability。Global schema-v3 evidence 與 immutable Store-v2 receipt
  evidence 明確分離，沒有把同一 receipt 重綁到新 manifest。
- **One Writer**：transaction 會重驗 ACTIVE daemon authority、epoch 與 locked physical
  head，但 TASK-021 不聲稱 Writer Lease／Codex product-writer composition 已完成；
  這些仍屬後續 ticket。
- Task Ledger 2.1 唯一擁有 request/event/command receipt/resource projection/checkpoint/
  admission semantics；Postgres Store 1.3 只擁有 migration/catalog/ACL/client/transaction/
  locking/retry/poison/durability mechanics。沒有第二個 mutable semantic owner。
- `PostgresTaskLedger` 呼叫 Task Ledger public planner/verifier，但不呼叫另一個 concrete
  adapter；Store 與 Ledger 兩個 SQL finalizer 是同一 transaction 內的兩個 fixed
  function call，不是兩個 commit 或兩個 truth。
- Store receipt 只證明 physical persistence；它不單獨證明 domain currentness、effect
  delivery、Guardian authority、release safety 或 provider success。

## Initial Architecture Findings And Resolutions

第一次 architecture review 結論為 `BLOCK`：P0=0、P1=1、P2=1、P3=0。

### P1 — Current-transaction atomicity 未被充分保證

Ledger finalizer 原本只檢查 matching Store terminal values，卻未證明 terminal 是本次
外層交易所建立。較早 Store-only terminal 可能被後續 Ledger call 靜默接成完整 pair，
使「兩個 fixed finalizers、一個 transaction」的架構承諾不成立。

修復在 `0004_task_ledger_repository.sql:2309-2345` 以
`xmin = pg_current_xact_id()::xid` 要求 current-transaction terminal，再核對 scope、
request、record set、checkpoint、receipt、outbox commitment、revision 及 state。
PostgreSQL 17.10 live primitive 證明同交易為 true、commit 後新交易為 false；SQL static
contract 同時固定 finalizer 必須包含這個條件。Store-only partial data 只能 fail closed，
沒有 auto-repair 或第二個 writer。

### P2 — Bounded failure 只有文字承諾

Runtime transaction 與 SECURITY DEFINER functions 原本沒有 lock/statement timeout，
外部鎖可能讓 daemon 無限等待，與 bounded transaction／可停止性要求不一致。

修復在 Rust read/write transaction settings 與八個 v3 runtime functions 同時固定
`lock_timeout=5s`、`statement_timeout=30s`；schema verifier 核對 `proconfig`，SQLSTATE
`55P03`/`57014` 映射成 terminal `Unavailable`。Live held-lock regression 證明逾時且
沒有狀態異動。這不新增 scheduler、background thread 或額外服務依賴。

兩項 architecture finding 都已修正並重新審查，沒有 risk acceptance（風險接受）。

## Verification-Discovered Boundary Repairs

初審修復後，實際 PostgreSQL harness 另外阻止了兩個錯誤假設進入最終架構：

1. **Fresh genesis mismatch**：Ledger finalizer 曾無條件要求 Store genesis digest 等於
   vacant Ledger checkpoint。修復把 existing-stream base-check 放在
   `v_stream_found` 分支，fresh 路徑只接受完整 structural zero、physical revision 0，
   並要求 after state 等於 next checkpoint（`0004:2274-2345`）。因此 Store physical
   genesis 與 Ledger checkpoint 維持兩個明確契約，不靠偶然 hash equality 耦合。
2. **Restart fixture admission**：initial 測試曾留下 ACTIVE，restart verifier 正確
   fail closed。測試收尾改回 STOPPED/no-leader，而非放寬 schema/admission verifier；
   service lifecycle 與 migration gate 的責任仍然分離。

最終 code re-review 隨後找到 vacant load 的 wrong-scope orphan P2。修復後
`task_ledger_read_head_v1(bytea,text,text)` 不論 Ledger stream 是否已存在，都會以
expected project/snapshot 全域檢查相同 owner+aggregate 的 physical collision；Rust 不再
從不完整 scope 推測結果。這保留 project/snapshot/stream isolation，而沒有引入跨專案
lookup capability 給公開 API。

## Contract, Migration, And Compatibility Result

- `0001` 至 `0003` byte-identical（位元組完全相同）；唯一 expansion migration 是
  transaction-control-free `0004`，最終 111,742 bytes、SHA-256
  `cd658ed2f4624cd0a829c818c1cf96d8ac3829264046976bdde3b2fc7feea6e5`。
- 四筆完整 manifest commitment 是
  `09c431df18ad71a4f44239a5d2ddf6b1774b8ffec06c7f9223f0e41757f3d407`；Store-v2 receipt
  永遠保留 profile 2 與 first-three-entry manifest
  `4582edce68a947998a8f4c6895bb37ceec9e842f516471f4d9e2617a6757f129`。
- Runner 只接受 Fresh、exact empty v1 prefix、exact STOPPED/no-leader v2 prefix 或
  exact v3 full state；partial、edited、reordered、unknown、ACTIVE 或 catalog drift
  全部 fail closed。沒有 destructive downgrade（破壞性降級）。
- `0004` 精確新增四個 Ledger/outbox tables 與八個 functions；沒有 composite/table
  argument type、generic row surface、dynamic SQL 或第九個 runtime function。
- Ledger `u64` 透過 constrained `numeric(20,0)` 與 canonical decimal text 保存；Store
  physical revision 保持 checked signed `BIGINT`。受限 diagnostic `jsonb` 不作 hash
  input，Rust 會重建並驗證。
- Exact retry／changed reuse 早於 mutable admission；new terminal command 每次只改變
  complete checkpoint 一次。Denial 沒有 event/outbox；只有 appended recorded effect
  intent 有一個 admission。

## Dependency And ACL Result

- `lattice-contracts` 保持 zero dependency；`lattice-ports` 只依賴 Contracts。
- `lattice-postgres-store` 對 `lattice-task-ledger` 的單向 edge 是已核准的 domain-plan
  consumption，不是 concrete adapter coupling。沒有 dependency cycle、reverse domain
  edge、OpenClaw/Codex/Graphify/Hermes dependency，或陪玩網站耦合。
- V3 catalog signatures 已由實際 PostgreSQL 17.10 與 verifier 對齊：
  relation `85619233866577a32550fac8f83f9995c05f24ddeaaf64d9563609e6c9ac8767`；
  column `7cd1aa5142dbccdc2ac2db466ba4ffdf0c9c41a1000ae0c59baf650e36bbaae8`；
  constraint `f9d587125d792646b77ca68e6224c9866bd32c87a0e98c4d2f85b75dd0c22be8`；
  index `40ca5ea0781b1be03efe9bead50ae9f78434314123d6f700d278874678d06a9b`；
  function `f2c8585e1da944b38a50c65c6b9f448963f4c3d96c909331be87fec0c30d2279`；
  function ACL `579b843df8e187eb0f4b7a75e9d1b0c4f109d596c55bcff5aa76a1a06bfcd91b`；
  table ACL `27a0879d1b709abd341653b445d3a64d59819bde2e20e868ac09d2624aab1993`。
- 八個 functions 均為 migrator-owned、`SECURITY DEFINER`、non-leakproof、
  parallel-unsafe、schema-qualified、safe search path、row-security-on、fixed timeout；
  runtime grant 精確且不可轉授。六個 protected tables 對 runtime/PUBLIC/Guardian/
  reader/pre-SET ROLE logins 維持零直接 SELECT/DML。

## Failure, Rollback, And Changeability Result

- Domain planning 保持在 pure Task Ledger；adapter complexity（配接器複雜度）集中於
  Postgres Store，不把 SQL/Client 型別洩漏到 Ports 或 Contracts。
- Store finalizer 先執行但尚未 commit；Ledger finalizer 或後續步驟失敗會由同一外層
  transaction 回滾兩邊。沒有 partial commit 或補寫成功假象。
- Serialization/deadlock/fixed first-row race 只有最多三次 pre-commit retry；明確 DB
  failure 終止，沒有 response 才是 unknown。Unknown 會 poison instance，透過新 client
  exact replay 收斂。
- Coherent global manifest drift 即使發生在 adapter 建構後，也會由每次讀寫的 SQL
  evidence 與 constructor-frozen evidence mismatch 拒絕。
- Wrong-scope、outbox linkage、head/checkpoint、terminal pairing 或 retained row corruption
  都 fail closed；沒有通用 repair API。這使 failure mode 可觀察且不建立第二個真相。
- 完整 per-append replay 目前集中在這個 adapter，未形成跨模組 shotgun surgery；後續
  若需要 snapshot/pagination，必須量測並版本化，不得暗改現有 checkpoint contract。

## Evidence Reviewed

- `PLANS.md` Step 6、SPEC-002 v23、ADR-019、TASK-021、governance review、Task Ledger
  2.1 與 Postgres Store 1.3 constitutions。
- Task Ledger source/tests；Postgres Store live/migration/setup/Ledger adapter；`0001` 至
  `0004`；migration contract；PostgreSQL live harness；Cargo manifests/lock/dependency tree。
- Final engineering evidence：PostgreSQL 17.10 marker-owned initial/restart harness PASS、
  432/432 Rust tests、44/44 Node tests、strict format/Clippy、109-dependency RustSec audit、
  catalog/ACL verification、scope/secret/dynamic-SQL scans 均 PASS。
- Code/security re-review 最終 P0=0、P1=0、P2=0、P3=0；fresh genesis、timeout、
  current-transaction terminal、outbox linkage、manifest drift、commit classification 及
  existing/vacant wrong-scope regressions 都有直接或組合證據。

Passing tests 被當作 implementation evidence（實作證據），沒有取代 responsibility、
ownership、dependency、migration 與 failure-path 的獨立架構判斷。

## Amendment And Governance Result

- Confirmed architecture violations：無。
- Open P0-P3 findings：無。
- Required ADR：無；ADR-019 已明確定義 transaction ID、Store mapping、outbox admission、
  ordered finalizers、frozen receipt profile 與 failure outcomes。
- Required constitution amendment：無；Task Ledger 2.1 與 Postgres Store 1.3 已涵蓋
  最終責任、依賴、資料所有權、公開契約及 acceptance gates。
- 新增 human architecture decision（人工架構決策）：無。這不代表 primary-branch
  merge、正式啟用或 protected release 可以略過各自權限。

## Residual Risks And Deferred Work

- 70-argument finalizer 的 current-transaction 條件使用 static SQL contract 加
  PostgreSQL 17 `xmin` live primitive；未額外複製一個脆弱 raw 70-argument call。這是
  殘餘證據維護風險，不是架構阻擋。
- Stream replay 在目前 MVP-1 slice 可能隨歷史線性成長；效能、snapshotting 或 pagination
  尚未量測／核准。未來變更必須版本化且保留 complete-checkpoint invariant。
- TASK-021 只 durable-admit effect intent；claim、delivery、retry、reconciliation、live
  resource observation、other repositories、Writer Lease composition 與 exactly-once effect
  均保持 open。
- Marker-owned loopback PostgreSQL 17.10 不是 production provisioning；credential、
  remote/TLS、installed-service replacement、provider/product runtime、Guardian activation、
  release 與 deployment 均未實作。
- Remote CI、branch protection、required review enforcement、upstream、committed candidate、
  merge authorization 仍 missing/unverified。Local PASS 不等於 merge readiness。

## Integration Blocker Result

`NO ARCHITECTURE BLOCKER`。最終結果為 P0=0、P1=0、P2=0、P3=0；TASK-021 可進入
獨立 local integration review（本機整合審查）。本報告不授權或聲稱 commit、push、
primary-branch merge、production migration、activation、release 或 deployment。
