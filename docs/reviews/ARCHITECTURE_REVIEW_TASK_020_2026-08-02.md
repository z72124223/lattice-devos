# TASK-020 Architecture Review

## Decision

`PASS`。最終獨立架構審查未發現剩餘 P0、P1、P2 或 P3 finding
（問題）；TASK-020 沒有 architecture integration blocker（架構整合阻擋）。

本結論只接受 TASK-020 的 physical Store（實體儲存交易邊界）範圍，
不把它擴張解讀為 domain repository（領域儲存庫）、One Writer（單一寫入者）
組合、provider effect（外部元件效果）、Guardian activation（守護者啟用）、
正式環境、發布或部署完成。

## Review Triggers

本次變更同時命中公開 contract（契約）、port（抽象介面）、持久化 schema
（資料庫結構）、migration（遷移）、安全權限、可靠性與失敗復原語意，因此必須
執行架構審查：

- Contracts 1.8 -> 1.9 新增 Store v2 live/durable representation
  （即時／耐久表示），並保留 v1 fake-only compatibility（假實作相容性）。
- Ports 1.3 -> 1.4 將 `ControlStore::current_head` 改為明確的 `&mut self`，
  以反映同步 PostgreSQL client（用戶端）的真實可變性。
- Postgres Store 1.1.5 -> 1.2 新增 schema v2、三個固定 runtime function
  （執行期函式）與 `PostgresControlStore`。
- `0003_live_control_store.sql` 改變持久化 evidence（證據）欄位、constraint
  （限制條件）、function ACL（函式存取控制）及 v1 -> v2 升級路徑。

## Before And After Architecture

| 面向 | TASK-019 以前 | TASK-020 最終狀態 |
|---|---|---|
| Contracts | Store v1；只能表達 `Fake` / `NonDurableFake` | v1 原樣保留；Store v2 可表達完整 `Live` / `DurablePostgres` persistence evidence |
| Ports | driver-free `transact` 與不可變 `current_head(&self)` | error/transaction surface 不變；只把同步查詢改為 `current_head(&mut self)` |
| PostgreSQL | exact schema v1、STOPPED/no-leader、無 live Store function | exact schema v2、三個固定 function、runtime 無直接 physical/terminal table access |
| Live adapter | 不存在 | caller-supplied、already-authenticated `Client`；constructor 先做 exact runtime schema verification |
| Transaction | 無 live durable receipt | prepare/finalize 位於同一個 bounded `SERIALIZABLE` transaction；commit 後才回傳 durable receipt |
| Migration | fresh v1 foundation | `0001`/`0002` 不變；fresh target 或 verified empty exact-v1 prefix 才可進入 v2 |
| Domain ownership | Registry/Ledger/Lease/Approval/Artifact 各自擁有合法性 | 完全不變；Store 只持有 opaque commitment（不透明承諾）與 physical CAS head |

## Boundary And Ownership Result

- **One Gateway**：TASK-020 沒有新增 OpenClaw、IPC、一般人機入口或另一個
  orchestration surface（編排介面）。
- **One Truth**：PostgreSQL 是唯一 live durable physical truth；fake 仍明確標示
  `RuntimeKind::Fake` / `NonDurableFake`，不得冒充 durable truth。
- **One Writer**：Store rechecks（重新核對）完整 ACTIVE daemon instance、epoch、
  authority revision/digests 與 locked physical head，但沒有聲稱這已完成 Writer Lease
  或 Codex product-writer composition；後者仍屬後續 domain ticket。
- Mutable data ownership（可變資料所有權）仍唯一：Contracts 只擁有 immutable
  representation；Ports 只擁有 trait/error；Postgres Store 只擁有 physical
  persistence/transaction mechanics；各 domain module 保留 transition legality。
- Store receipt 只證明綁定 database/schema/request/head 的 opaque physical transaction
  已提交；它不證明 domain legality/currentness、effect delivery、Guardian authority
  或 release safety。

## Dependency Result

`cargo metadata --locked --no-deps --format-version 1` 的 direct dependency
（直接依賴）結果：

- `lattice-contracts`: 無依賴。
- `lattice-ports`: 只依賴 `lattice-contracts`。
- `lattice-postgres-store`: 只依賴 `lattice-cjson`、`lattice-contracts`、
  `lattice-ports`、exact `postgres` 與 exact `sha2`。

依賴方向與三份 module constitution（模組憲章）一致；未形成 cycle（循環依賴）、
reverse domain dependency（反向領域依賴）或 adapter-to-adapter dependency。
Store source/migration 的 bounded scan（範圍掃描）沒有 OpenClaw、Codex、Graphify、
Hermes、provider、product、website、playmate/陪玩或其他 domain crate 外溢。

## Migration And Compatibility Result

- `0001_bootstrap.sql` SHA-256 保持
  `7bff021fc17f738551309c906578c8015b2dd0307d27d239c21df1697c4d09c8`。
- `0002_control_store_foundation.sql` SHA-256 保持
  `e996dc64af3112a647e75ebf07df2a77b1e9b3a018ed443880150365184883f0`。
- `0003_live_control_store.sql` 是唯一新 expansion migration（擴張遷移），
  最終 SHA-256 為
  `00ae3eedd76704f26b1df58955d9d594c98f0ba525be93b15d8c9ebb1f2115c1`。
- Runner（遷移執行器）只接受 fresh、verified empty exact-v1 prefix 或 exact-v2
  full state；edited、partial、reordered、unknown、non-empty 與 catalog drift 均
  fail closed（失敗即拒絕）。
- v1 source verification、missing-entry apply、history/compatibility advance 位於同一個
  migration transaction；失敗回滾，不宣稱 destructive downgrade（破壞性降級）。
- Store v1 request/receipt 的 fake-only meaning 保留；live adapter 僅接受 v2 + Live。

## Security And Failure Boundaries

- Schema v2 只建立 `store_prepare_v2`、`store_finalize_v2`、
  `store_current_head_v2` 三個 Store function；migration 內無 dynamic SQL
  （動態 SQL）。
- Runtime 對 physical/terminal tables 維持零直接 SELECT/DML；只有固定 function
  EXECUTE。`PUBLIC`、Guardian、reader 與 pre-`SET ROLE` LOGIN 無執行權。
- SECURITY DEFINER（以擁有者權限執行）函式採 schema-qualified relation、固定
  `search_path = pg_catalog`、row security、固定 signature/source/property/ACL
  verifier，沒有 generic SQL/JSON row escape hatch。
- Exact replay/changed-ID classification 先於 mutable admission/head check；changed
  content 不回傳 retained receipt。
- Applied head 與 terminal receipt 同 transaction 出現；stale receipt 不改 head，
  first-use stale 也不 materialize genesis（寫入虛擬初始 head）。
- Serialization/deadlock/fixed first-row race 只做 bounded pre-commit retry；未知 commit
  response 回傳 `CommitOutcomeUnknown`、poison 原 instance，且只能由新 client 加 exact
  request 重連對帳。
- `PostgresControlStore` 公開 live API 只有 `new(Client, MigrationTarget)` 加上
  `ControlStore` trait；沒有 raw client getter、arbitrary query、DSN、password、
  environment 或 connection discovery。

## Findings And Resolutions

審查期間曾找到一個 P1 governance inconsistency（治理不一致）：Postgres Store 1.2
constitution 的舊 Non-Goal 文字曾同時禁止 live durability claim，與同檔 Mission、
Public Contracts、SPEC-002 v22、ADR-018 及 TASK-020 相衝突。主工作流已先修正為：
physical live durability 不能單獨證明 One Writer、domain、Guardian、effect 或 release。
Ports trait 與 crate/module documentation 的舊 fake-only/TASK-019 wording 也已同步。

重驗後：

- Confirmed violations（已確認違規）：無。
- Open P0-P3 findings：無。
- Required ADR or constitution amendment（必要 ADR／憲章修訂）：無；現有 accepted
  ADR-018 與 version 1.2 constitution 已涵蓋最終實作。
- Human architecture decision（人工架構決策）：無新增需求。

## Residual Risks And Deferred Work

- TASK-020 只完成 AC-34 physical Store slice；AC-03/04/05/19 與 MVP-1 domain
  repositories、outbox/filesystem effect、Writer Lease composition 仍保持 open。
- Live target 仍刻意限制在 marker-owned、loopback、PostgreSQL 17 disposable cluster；
  production provisioning、credential、remote/TLS 與 installed-service replacement 未實作。
- Test-admin ACTIVE fixture 只屬測試；normal runtime 沒有 self-activation/election API。
- Structural receipt constructor（結構型收據建構器）本身不是真實 durability proof；
  composition root 必須注入已驗證的 `PostgresControlStore`。這是既定 Contracts/Ports
  boundary，不是本次 finding。
- Local test PASS 不能代替 remote CI、branch protection、merge authorization 或
  production acceptance；TASK-020 不授權 commit/push/merge/deploy。

## Evidence Reviewed

- `PLANS.md` current Step 6、SPEC-002 v22 AC-34、TASK-020。
- ADR-018、Contracts 1.9、Ports 1.4、Postgres Store 1.2 constitutions 與 V2 amendment。
- Contracts/Ports/Store source、Cargo manifests/lock、exact migration manifest、
  `0003` SQL、migration/runtime verifier、fake/live tests 及 disposable harness scope。
- Direct checks: Cargo metadata、migration SHA-256、exact three-function count、
  no-dynamic-SQL scan、forbidden dependency/scope scan、current governance wording scan。
- Final engineering evidence supplied to this review: PostgreSQL 17.10 disposable harness,
  409 Rust tests、44 preserved Node tests、strict Clippy/format 與 `cargo audit` 均 PASS。
  Passing tests were treated as implementation evidence, not as a substitute for this
  ownership/dependency/failure-path review.

## Integration Blocker Result

`NO ARCHITECTURE BLOCKER`。TASK-020 可進入獨立 integration review（整合審查），
但最終 merge readiness（合併就緒）仍須由 repository policy、同步、CI、review 與
明確 merge authorization 分別判定。
