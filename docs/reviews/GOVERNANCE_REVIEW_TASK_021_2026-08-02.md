# TASK-021 Governance And Architecture Review

## Decision

`PASS — AMENDED FINALIZER DESIGN PRESERVES THE IMPLEMENTATION GATE`。

最新治理快照沒有未解決的 P0 或 P1 finding（問題），也沒有需要新增人工決策的
架構阻擋。TASK-021 可以依既定 RED/GREEN 順序開始實作；本結論只允許票券內的
可逆本機程式碼、測試與 marker-owned disposable PostgreSQL（標記擁有的拋棄式
PostgreSQL）工作，不代表實作、AC、MVP、合併、正式環境、發布或部署已完成。

## Reviewed Scope

- `PLANS.md` Step 6 與唯一 `CURRENT TASK-021 GOVERNANCE` 標記。
- SPEC-002 v23，特別是 AC-03、AC-04、AC-24、AC-35 與明確 deferral
  （延後範圍）。
- ADR-019、已核准的 `V2_AMENDMENT_PROPOSAL.md`、Task Ledger 2.1 與
  Postgres Store 1.3 constitutions（模組憲章）、TASK-021 與其 workflow audit
  （工作流稽核）。
- TASK-020 ticket、ADR-018、最終 code/security review（程式碼／安全審查）、
  architecture review（架構審查）、integration review（整合審查）與 closure
  evidence（結案證據）。
- 目前 Task Ledger 2.0 public API/replay/fake、Store v2 contracts、
  `PostgresControlStore`、migration manifest（遷移清單）、schema verifier
  （結構驗證器）、`0001` 至 `0003` SQL 與 live PostgreSQL harness（即時資料庫
  測試驅動器）。

## Architecture And Ownership Result

| Boundary | TASK-020 baseline | TASK-021 governed result |
|---|---|---|
| Task Ledger | Pure event/request/head/receipt/resource replay and fake | Remains pure and zero-I/O; adds shared vacant/plan/apply, retained commands, complete checkpoint, and conditional outbox admission |
| Postgres Store | Physical Store v2 transaction and durable receipt | Adds one Task-Ledger-planned durable adapter, global schema v3, and frozen historical Store-v2 receipt profile |
| Mutable semantic ownership | Domain owner only | Unchanged; SQL persists and locks but does not construct Ledger meaning |
| Adapter composition | One concrete physical Store | `PostgresTaskLedger` shares crate-private physical mechanics/functions and does not call `PostgresControlStore` or another adapter |
| Durable truth | PostgreSQL physical heads/receipts | PostgreSQL atomically retains Ledger rows/checkpoint plus the matching physical receipt |

Task Ledger 2.1 alone owns command, event, receipt, projection, checkpoint, replay,
and outbox-admission semantics. Postgres Store 1.3 owns only SQL, catalog, ACL,
client/transaction, locking, static conversion, retry/poison, migration, and durability
mechanics. There is no reverse dependency and no second mutable owner.

## Contract, Idempotency, And Checkpoint Result

- Exact `(stream_id, command_id)` retry and changed-content classification occur
  before mutable admission. Exact retry returns the retained result without depending
  on the current daemon epoch/admission/head; changed reuse returns no receipt.
- Every new terminal command, including stale/overflow denial, changes the complete
  Ledger checkpoint and applied physical Store state exactly once. A denial creates no
  event/outbox and leaves the Ledger event head unchanged.
- The checkpoint binds complete identity/runtime, full head/resource projection,
  ordered events, every appended or denied command, and ordered outbox admissions.
  Plan application rechecks the complete base checkpoint.
- `task_ledger_streams` must persist all authoritative head/projection fields as
  `NOT NULL`. A stream with denials before its first event stores the complete
  Task-Ledger-derived structural-zero head; nullable head state is forbidden.
- The globally unique Store transaction ID is frozen as
  `task-ledger-v1:<sha256>` over domain
  `lattice.postgres-task-ledger.store-transaction-id` v1.0 and fixed
  `TASK_LEDGER` owner, full stream ID, and full command ID. It is stable across
  restart/unknown commit and cannot collide through raw-ID truncation.
- ADR-019 exhaustively maps Ledger identity and plan values into `StoreScope` and
  `StoreMutationCommitment`: owner/scope, request, record set, next checkpoint,
  terminal receipt, checkpoint, and optional admission digest are no longer left to
  adapter interpretation. The admitted row separately retains the original intent
  digest equal to `LedgerEvent::subject_digest()`.

## Outbox Result

The admission rule is now unambiguous and compatible with Task Ledger 2.0:

1. the command terminal result must be `CommandOutcome::Appended`;
2. the event kind must be `LedgerEventKind::EffectIntent`;
3. the audit outcome must be `LedgerOutcome::Recorded`.

Only that combination derives exactly one immutable `ADMITTED` outbox record.
Existing appended `EFFECT_INTENT` events with another audit outcome remain valid and
retain their existing hashes/behavior, but derive no admission. A stale/overflow-denied
command or any non-effect event also derives none. This preserves compatibility without
allowing `FAILED`, `DENIED`, `BLOCKED`, or `CANCELLED` audit events to become executable
effect intent.

## Migration, Function, ACL, And Profile Result

- `0001` through `0003` remain immutable. Exact transaction-control-free `0004`
  alone advances global compatibility to schema v3 and may add only four Ledger/outbox
  tables plus eight new fixed functions.
- Fresh, exact v1 prefix, exact v2 prefix, and exact v3 full state are the only accepted
  runner states. Non-empty v2 requires exact source proof plus `STOPPED`/no-leader;
  ACTIVE, partial, edited, reordered, unknown, or drifted state fails closed.
- Runtime may execute only three Store-v3 and five Task-Ledger-v1 functions and has
  zero direct protected-table SELECT/DML. The three historical Store-v2 functions
  remain catalog history with zero runtime EXECUTE.
- All runtime functions remain fixed-signature, schema-qualified, dynamic-SQL-free,
  migrator-owned `SECURITY DEFINER`, non-leakproof, parallel-unsafe, safe-search-path,
  and row-security-on surfaces with exact non-grantable runtime grants.
- Global schema-v3 evidence is separate from the immutable Store-v2 receipt profile.
  Historical and new Store v2 receipts continue to bind physical profile 2 and the
  first-three-entry manifest commitment
  `4582edce68a947998a8f4c6895bb37ceec9e842f516471f4d9e2617a6757f129`.
- Ledger `u64` fields use constrained `numeric(20,0)` and canonical decimal text;
  physical Store revision remains checked signed `BIGINT`. Only bounded sanitized
  diagnostic data may use `jsonb`, and PostgreSQL JSON text is never hash input.

## AC-24 And Dependency Interpretation

AC-24 remains valid. `lattice-ports` is unchanged and still depends only on
`lattice-contracts`. The approved new edge is
`lattice-postgres-store -> lattice-task-ledger`; Task Ledger is the pure semantic
domain owner, not another concrete adapter. There is no adapter-to-adapter call,
Orchestrator dependency, reverse domain dependency, cycle, or new crate. The edge
therefore does not reopen or contradict AC-24.

## AC-03 And AC-04 Closure Boundary

TASK-021 may close AC-03 only after direct PostgreSQL evidence proves atomic command
idempotency, expected head/sequence/predecessor/event validation, event/head update,
conditional outbox admission, terminal receipt, checkpoint, and physical receipt.
Outbox claim/delivery is not part of AC-03; AC-03 requires the durable intent/admission
to exist before any later external effect begins.

TASK-021 may close AC-04 only after restart replay reproduces the same Ledger-owned
head/resource projection/checkpoint and every corruption/mismatch matrix fails closed.
A live resource observation/owner-currentness receipt authorizes a future effect but
does not define Ledger persistence replay. It can therefore remain deferred together
with effect claim/delivery, Task Domain/Orchestrator composition, and other repositories.
AC-05, AC-19, MVP-1, MVP-2, and MVP-3 remain open.

## Finalizer-Sequencing Amendment Review

`PASS`。PostgreSQL 的 function-argument limit（函式參數數量限制）使單一巢狀
Store+Ledger scalar finalizer（純量完成函式）不可行，而 table/composite row
arguments（資料表／複合列參數）會引入額外型別與權限歧義。ADR-019、Postgres
Store 1.3、TASK-021 與 `PLANS.md` 改採的兩個固定函式順序沒有新增 P0/P1：

- Rust 仍只開啟一個外層 `SERIALIZABLE` transaction（可序列化交易），先呼叫
  `store_finalize_v3`，再呼叫 `task_ledger_finalize_v1`，且只在兩者成功後 commit。
  這是兩次函式呼叫，不是兩次提交。
- Store finalization 產生的 physical head/terminal 在同一未提交交易內立即可供
  Ledger finalizer 驗證，但對其他 session 不可見。Ledger finalizer 重新核對完整
  base checkpoint、deterministic transaction ID、Store request/terminal 與固定
  row-count invariants，之後才寫入 Ledger command/event/outbox/stream rows。
- 第二個函式或其後任何步驟失敗會使外層交易回滾，因此先前 Store finalization
  也一併消失；不會留下已提交的 Store-only half（單邊狀態）。既有 partial-pair
  規則仍要求發現舊有單邊資料時 fail closed，禁止把它當成本交易的新 terminal
  靜默補完。
- Semantic ownership（語意所有權）不變：Task Ledger planner 仍唯一建構與驗證
  command/event/receipt/checkpoint/outbox 意義；Store 函式只處理 physical receipt，
  Ledger 函式只持久化並重驗 planner 產物。
- ACL（存取控制）不變：runtime 仍只有既定三個 Store-v3 與五個
  Task-Ledger-v1 fixed-function EXECUTE，對 protected tables 維持零直接
  SELECT/DML。沒有新增 composite/table argument、`pg_type` capability、type/table
  grant、generic row surface 或第九個 runtime function。

因此 atomicity（原子性）、ownership（所有權）、exact replay/changed-ID ordering
（精確重放／變更 ID 順序）、commit-unknown/poison（提交結果未知／實例封鎖）與
Store-v2 frozen profile（凍結設定檔）均保持原治理語意。

## Resolved Pre-Code Findings

The review found and the owning workflow corrected these items before implementation:

1. Ambiguous `EFFECT_INTENT` outcome semantics were narrowed to appended + `RECORDED`,
   while preserving all existing non-`RECORDED` append behavior without admission.
2. Physical transaction identity gained a frozen domain-separated, cross-stream and
   cross-owner-safe derivation instead of an unspecified or truncatable ID.
3. Every `StoreScope`/`StoreMutationCommitment` field gained one exact Ledger mapping;
   admission digest and original intent digest are explicitly distinct.
4. The first-denial stream representation now requires a complete non-null structural
   zero head rather than nullable or invented-event state.
5. The impossible implication that a Ledger command existed before its schema-v3 tables
   was removed. Domain retry is tested within v3; historical Store-v2 replay is tested
   separately across v2-to-v3 upgrade.
6. SPEC-002's Module Impact table now agrees with its frontmatter and constitutions on
   Task Ledger 2.1.
7. Task Ledger 2.1's acceptance-gate row now matches its invariant and ADR exactly:
   admission requires appended `EFFECT_INTENT` + `RECORDED`; appended
   non-`RECORDED`, denied, and non-effect commands produce no admission.
8. An initially nested Store-inside-Ledger finalizer design exceeded PostgreSQL's
   scalar function-argument limit. It was replaced by ordered fixed Store/Ledger
   finalizers in one outer transaction, with exact cross-check and rollback of both,
   without composite arguments or broader runtime privileges.

No unresolved P0 or P1 finding remains.

## Residual Risks And Enforcement Truth

- TASK-021 implementation is now in progress. This narrow governance amendment review
  cannot by itself prove transaction, catalog, ACL, concurrency, restart, fault,
  corruption, or historical-replay behavior; the ticket's focused/live/full tests and
  independent final reviews remain mandatory.
- Complete per-append stream replay is intentionally simple for this MVP-1 slice and may
  become linear in stream history. Performance limits, snapshotting, or pagination need
  measured evidence and a future versioned amendment; they do not relax current
  correctness or fail-closed requirements.
- The generic physical Store can detect but not repair a Store/Ledger mismatch. The
  Ledger adapter must fail closed, and no auto-repair is authorized.
- Live resource observation, outbox claim/delivery/reconciliation, Writer Lease,
  production provisioning, remote/TLS use, provider/product execution, Guardian,
  release, and deployment remain explicitly deferred.
- Local checks are machine-enforced only for the current filesystem snapshot. Review,
  module ownership, and ticket scope are documented-only without remote required-review
  enforcement. Rust/PostgreSQL remote CI, branch protection, upstream synchronization,
  a committed candidate, and primary-branch merge authorization remain missing or
  unverified.

## Verification And Implementation Gate

Latest governance checks on the reviewed snapshot:

- `npm.cmd run check` -> PASS:
  `check=ok files=264 constitutions=18 tickets=21 current_tasks=1`.
- `git diff --check` -> PASS.

Implementation gate: start with the first focused failing Task Ledger test, then execute
TASK-021 one verified TDD behavior at a time. If implementation needs another table,
function, dependency, hash field/domain, receipt/profile meaning, mutable owner, failure
outcome, or deferred capability, stop and amend the SPEC/ADR/constitution/ticket before
coding. There is no governance blocker to local implementation; merge readiness remains
blocked and outside TASK-021.
