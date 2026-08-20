---
ticket_id: TASK-044
spec_id: SPEC-002
spec_version: 35
module_id: latticed
constitution_version: 1.9
status: complete
parallel_safe: false
depends_on:
  - TASK-038
  - TASK-050
  - TASK-075
  - TASK-076
allowed_paths:
  - docs/tickets/TASK-044-component-fault-recovery-gate.md
  - scripts/run-task044-component-fault-recovery-gate.ps1
  - scripts/test-task044-component-fault-recovery-gate.ps1
  - scripts/run-task019-postgres.ps1
  - apps/lattice-runtime/src/task_control.rs
  - apps/lattice-runtime/tests/task_control.rs
  - docs/modules/latticed/MODULE_CONSTITUTION.md
  - PLANS.md
  - HANDOFF.md
likely_files:
  - docs/tickets/TASK-044-component-fault-recovery-gate.md
  - scripts/run-task044-component-fault-recovery-gate.ps1
  - scripts/test-task044-component-fault-recovery-gate.ps1
  - scripts/run-task019-postgres.ps1
  - apps/lattice-runtime/src/task_control.rs
  - apps/lattice-runtime/tests/task_control.rs
  - docs/modules/latticed/MODULE_CONSTITUTION.md
  - PLANS.md
  - HANDOFF.md
branch: feature/task-044-component-fault-recovery-gate
base_commit: 65f2902504e5ef5acba6f258b736905fd4d12a4d
---

# TASK-044 — Local Component Fault / Restart / Reconciliation Exit Gate

## Current Gate

`PASS / EXACT_IMPLEMENTATION_SOURCE f6e66dad6e8982d7a0d41692b96469cfbab98a04`

2026-08-20 修復重啟已把本 ticket 的單一設計提交搬到乾淨且與遠端一致的
`feature/task-076-postgres-writer-lease-v2@65f2902`。該基底包含 TASK-038 的
canonical-local task submit/status、TASK-050 的 fresh-`latticed` autonomy receipt
replay、TASK-075 schema-v5 reconciliation 與 TASK-076 Writer Lease v2 bridge；
原本的 shared-path checkpoint blocker 已解除。

本輪不改 Contracts、Ports、Orchestrator、MCP/OpenClaw ingress 或任何 component
adapter。除 aggregate runner 外，唯一 runtime 行為修復是讓既有 PostgreSQL
`TaskLifecyclePort` 接受 Task Domain 已合法的 pre-writer `Draft -> Cancelled` transition；
它必須在 required autonomy receipt 完成後、沒有 Writer Lease authority 時執行，且不
新增 submit/stop 工具、active-child interrupt 或 live OpenClaw authority。

實作範圍包含一個 fail-closed aggregate runner 及其靜態／行為自測：runner 只能把
既有 owner tests 與 marker-owned PostgreSQL acceptance 的 current execution evidence
彙整成 TASK-044 證據；缺少、skip、解析失敗、未清理或未證明的情境一律不得輸出
總 PASS。這個 aggregate gate 不改寫 owner semantics，也不把歷史 PASS 當成 current
evidence。

2026-08-20 clean-source aggregate run
`e5784371df1a46c78a64b8f1cf9c59cd` 在 exact source `f6e66da` 完成 13 個 current
commands、8/8 scenarios、六個 fresh-status effect delta 全為零，並輸出：

```text
TASK044_COMPONENT_FAULT_RECOVERY_GATE=PASS
SCENARIOS=F01,F02,F03,F04,F05,F06,F07,F08
FRESH_STATUS_ZERO_RERUN=PASS
CLEANUP_FOOTPRINT=PASS
```

## Historical Blocker (superseded 2026-08-20)

下列限制只記錄 2026-08-10 的歷史狀態，不是目前執行閘門。

本 ticket 是 Step 11 Phase B 的 bounded harness design。2026-08-10 盤點時，
`feature/task-038-chatgpt-mcp` 仍有未提交的 Contracts、Ports、Orchestrator、
runtime、Writer Lease、PostgreSQL Store 與共用 harness 變更；在一個精確、乾淨、
可重播的 TASK-038 stable checkpoint 出現前，本 ticket 不得修改或複製那些路徑，
不得啟動 live PostgreSQL/WSL、full-chain runner、正式 paid/provider path，亦不得跑
full workspace matrix。

目前唯一允許的寫入是本 ticket。現有所有 worktree、root `PLANS.md`、
`HANDOFF.md`、`scripts/run-task019-postgres.ps1`、
`scripts/run-task037-full-chain-verification.ps1`、
`scripts/run-task038-task-submit.ps1`、runtime/Postgres paths，以及 TASK-043 均保持
不變。

## Objective

在同一個 marker-owned disposable run 中，對 PostgreSQL、Codex、Graphify、
Hermes、PostgreSQL Codebase Memory 與 fresh Status 建立一個 fail-closed 的
component fault/restart/reconciliation exit gate。每個情境必須證明：

1. 故障前的 durable intent/dispatch receipt 已精確綁定；
2. 故障不會被誤報為成功，第一個失敗會抑制後續 effect；
3. unknown outcome 只進入 typed reconciliation，不猜測成功或安全重跑；
4. PostgreSQL 與受控程序重啟後，fresh Status 只讀 single truth；
5. Status 不重跑 Codex、verification、Git、Graphify、Hermes 或 Memory；
6. stale snapshot、cross-project evidence 與 receipt substitution 被拒絕；
7. 所有 marker-owned process、listener、database、worktree、snapshot、staging、
   Hermes temp root 與 credential copy 都符合 exact cleanup allowlist；
8. One Gateway / One Truth / One Writer 與既有 domain owners 不變。

## TASK-043 Partition

- TASK-043 單獨擁有 `crates/lattice-codebase-memory/**` 的 pure retrieval benchmark、
  ranking/quality threshold 與其 ticket。
- TASK-044 禁止修改 `crates/lattice-codebase-memory/**`、
  `docs/tickets/TASK-043-codebase-memory-benchmark.md`，也不得複製 TASK-043 的
  dirty tree。
- TASK-044 只觀察已持久化 Memory receipt、row/count/digest footprint、
  project/commit binding 與 fresh-status zero-rerun；不重新定義 retrieval quality。
- Step 11 的總出口需要 Phase A（TASK-043）與 Phase B（TASK-044）各自通過，
  但兩者沒有共享程式碼寫入權。

## Classification

- `ME` — machine-enforced：已有直接失敗斷言的 test 或 fail-closed harness，且接到
  明確入口。此標記只表示 gate strength；本輪受資源門檻限制，未重新執行。
- `DO` — documented-only：SPEC/ADR/ticket 或程式分支描述行為，但沒有直接、可失敗
  的驗證入口。
- `MI` — missing：盤點中找不到足以證明該情境的機器斷言。
- `BL` — blocked：設計或 runner 已存在，但目前因 TASK-038 尚無 stable checkpoint、
  shared-path overlap、live-resource gate 或既有 production failure 而禁止執行/整合。

## Existing Evidence Index

| ID | Existing evidence at base `845328d` | What it actually proves |
|---|---|---|
| P1 | `scripts/run-task019-postgres.ps1:717-809,819-858` | marker-owned PostgreSQL initial/restart/multi-restart phases、stopped-state proof、exact cleanup 與 installed-service snapshot equality |
| P2 | `crates/lattice-postgres-store/tests/postgres_store.rs:549`; `crates/lattice-postgres-store/tests/postgres_live.rs:481,592,997,1338,2174,3308` | before/after-apply uncertainty、commit-response-loss reconciliation、bounded retry、lock timeout、Store/Ledger restart |
| C1 | `crates/lattice-codex-adapter/tests/process.rs:779,930,1092,1121,1147`; `protocol.rs:248,758` | preflight/readiness timeout、interrupt + owned-tree reap、descendant cleanup、EOF/malformed terminal denial |
| C2 | `crates/lattice-orchestrator/tests/delivery_order.rs:365-434`; `scripts/run-lattice-delivery.ps1:1309-1450,1738-1762` | ambiguous Git/DB/Codex result becomes reconciliation；deadline status survives DB restart；app-server invocation count remains one |
| G1 | `crates/lattice-graphify-adapter/tests/exact_git_snapshot.rs:26-139`; `wsl_bwrap_containment.rs:138-196`; `crates/lattice-orchestrator/tests/graph_memory_order.rs:102-247,289-415` | exact tracked commit snapshot、rejected snapshot leaves no partial directory、timeout/reap probe（ignored live WSL gate）、first-failure suppression、receipt-only status/cross-binding denial |
| H1 | `crates/lattice-hermes-adapter/tests/reflection_api.rs:1279-1356,1631-1760`; `hermes_sandbox_runner_fixture.py:503-522` | event transport loss -> bound status polling、same-process receipt reconciliation without resubmit、malformed reflection/timeout/unknown frame denial |
| M1 | `crates/lattice-postgres-codebase-memory/tests/postgres_live.rs:110-175,246-357,366-509,743-807`; `adapter.rs:462-567,745-747` | extension/receipt DB restart、exact replay、cross-project denial、partial profile rollback；commit-outcome-unknown codes exist but lack direct response-loss fault injection |
| S1 | `apps/lattice-runtime/tests/composition.rs:203-253`; `crates/lattice-orchestrator/tests/delivery_order.rs:434`; `scripts/run-lattice-delivery.ps1:1444-1565,1775-1784` | fresh Status does not fabricate process config、calls only durable receipt port、Graphify footprint unchanged、no Codex resend |
| F1 | `scripts/run-task037-full-chain-verification.ps1:870-876,1107-1187,1201-1239,1415-1425` | runner enforces pre-status failure、run/status equality、DB-restart replay、Memory count equality、process/listener cleanup；no successful current production E2E is established |

Historical TASK-033 evidence records a passing disposable PostgreSQL 17.10 run/status
restart with unchanged Graphify snapshot/staging footprint. It is retained evidence for
that exact scripted slice, not current TASK-044 or production full-chain acceptance.

## Exact Coverage Matrix

Cells classify current direct coverage, not intended architecture. `ME/BL` means a local
machine assertion exists but the combined live/full-chain proof is blocked.

| Scenario | PostgreSQL | Codex | Graphify | Hermes | Memory | Fresh Status |
|---|---|---|---|---|---|---|
| Dependency unavailable | `ME` P1/S1 | `ME` C1 | `ME` G1 error mapping/first-stop | `ME` H1 + sealed-runner denial | `ME` M1 missing/partial profile | `ME/BL` S1; production chain blocked |
| Timeout / cancel | `ME` timeout P2; `DO` task cancel | `ME` timeout/interrupt C1; `DO` durable cancel | `ME/BL` timeout mapping; `DO` cancel; live WSL reap is ignored and now blocked | `ME` event/status timeout H1; `DO` durable cancel | `DO` cancel/reconcile requirement | `DO` durable cancelled projection |
| Partial / malformed output | `ME` partial schema rollback P1/P2 | `ME` malformed/EOF C1 | `ME` G1 malformed/partial/foreign-source rejection | `ME` H1 malformed schema/frame | `ME` atomic rollback M1 | `ME` S1 strict envelope/property checks |
| Commit / ACK unknown | `ME` P2 | `ME` C2 timeout/ambiguous terminal and Git outcome | `DO` fresh-process reconciliation after ambiguous teardown | `ME` H1 only within same process | `DO` typed unknown-outcome branches; direct response-loss injection missing | `ME` C2 reconciliation replay for scripted delivery |
| Process restart | `ME` P1/P2 | `DO` mid-effect crash recovery; completed run has zero-resend only | `DO` mid-extract crash recovery | `DO` fresh-process broker/provider reconciliation | `ME` M1 DB/process replay | `ME/BL` S1 for TASK-033; full chain F1 blocked |
| Fresh Status zero-rerun | `ME` P1/S1 | `ME` C2 invocation count = 1 | `ME` S1 footprint digest unchanged | `BL` F1 has intended check but no accepted production result | `ME` M1/F1 row/digest equality | `ME/BL` exact scripted slice; combined gate blocked |
| Stale snapshot / project isolation | `ME` P2 project-bound request/head | `ME` request/evidence binding in delivery tests | `ME` G1 exact commit/cross-binding | `ME` H1 request/snapshot/receipt binding | `ME` M1 cross-project/revision denial | `ME` G1/S1 receipt cross-binding denial |
| Cleanup footprint | `ME` P1 | `ME` C1 | `ME/BL` fixture cleanup; live WSL/temp proof blocked | `DO` cleanup requirement; file-level temp-root machine proof missing | `ME` marker DB removal via P1 | `BL` F1 checks process/ports but not every component temp root |

## Overall Gate Classification

| Classification | Exact current state |
|---|---|
| `ME` | P1/P2、C1/C2、G1、H1、M1、S1 提供局部 fail-closed、restart、replay、cross-binding 或 cleanup assertions；TASK-033 的 exact scripted slice 有歷史 PASS，但本輪未重跑。 |
| `DO` | SPEC-002 MVP-2、ADR-021/022 與既有 tickets 要求 cancellation、unknown-effect reconciliation、component/process restart 與 safe cleanup；mid-effect Codex/Graphify/Hermes fresh-process recovery 目前仍只是規範。 |
| `MI` | 統一 F01-F08 dispatcher、durable cancel projection、Memory commit-response-loss fault injection、跨元件 effect counters、fresh Hermes no-resubmit proof、以及所有 temp/artifact roots 的 exact cleanup manifest 尚不存在。 |
| `BL` | TASK-038 shared contracts/harness 尚未有 stable checkpoint；live PostgreSQL/WSL、full-chain runner、full workspace matrix 與 production/paid provider path 依目前資源及安全門檻禁止執行。 |

## True Gaps

1. 沒有一個 scenario dispatcher 能在同一 run 逐一選擇上述八種情境，並把
   component/stage/injection point 綁到 durable receipts。
2. 沒有跨元件一致的 `effect_count_before_status` / `effect_count_after_status` 與
   file/process/listener/database footprint manifest。
3. timeout 有多個局部測試，但 durable task cancellation、取消後 terminal projection
   與 restart replay 仍缺機器 gate。
4. Memory adapter 有 commit-outcome-unknown 分支，卻缺 commit-response-loss 後的
   exact receipt reconciliation fault test。
5. Hermes 只有 same-process receipt reconciliation；fresh `latticed`/broker process 從
   PostgreSQL truth 回放且不 resubmit 的證據缺失。
6. Codex、Graphify、Hermes 在 effect-dispatched 後、terminal ACK 前被終止時，尚未有
   統一的 restart decision（query/reconcile/deny rerun）與 machine evidence。
7. TASK-037 cleanup 目前主要斷言 process/port；既有 incident 曾留下 Hermes temporary
   directories，因此 full file-level cleanup 不能宣稱完成。
8. TASK-038 的 task lifecycle、real Writer Lease/fence、fresh task status 與 shared
   runner contracts 尚未形成 stable checkpoint；現在修改 full-chain runner 會造成
   owner/contract 重疊。

## Bounded Scenario Contract

每次只注入一個 fault，使用全新的 marker-owned run。不得把 fake adapter 或文件存在
當成產品完成；production-shaped fixtures 只能證明 contract。正式 paid/provider path
永遠不在本 gate 內。

| ID | Injection | Required terminal and restart evidence |
|---|---|---|
| F01 | 依序使 PostgreSQL、Codex executable、Graphify runtime、Hermes broker/runtime 或 Memory extension 不可用 | 在目標 stage 前/中 fail closed；later-effect count = 0；fresh Status 回傳相同 typed failure/reconciliation，不建立替代 truth |
| F02 | 對 Codex、Graphify、Hermes、DB transaction/status query 施加 bounded timeout；對 admitted task 發出一次受控 cancel | timeout 必須區分 reaped-known 與 teardown-unknown；cancel 必須有 durable request/result，restart 後仍為 terminal cancelled/reconciliation，絕不變成 success |
| F03 | Codex JSONL terminal、Graphify graph、Hermes reflection、Memory/Status receipt 各提供一個 partial/malformed production-shaped fixture | parser/schema 拒絕；零 downstream persistence/commit；無 partial snapshot/staging/Memory success |
| F04 | 在 PostgreSQL/Memory COMMIT、Git commit、Codex/Hermes terminal ACK 邊界模擬 response loss | 若 durable side effect 可由 authoritative query 證明則載入 exact receipt；否則維持 reconciliation；不得盲目重送 |
| F05 | 在 durable dispatch receipt 後終止一個 marker-owned child，再重啟 `latticed`/必要 local component | one-writer/fence currentness 重新驗證；stale process 無 mutation；fresh process 只依 PostgreSQL 與 owning repository 決定 query/reconcile/deny |
| F06 | 完成 run 後停止 PostgreSQL 與 `latticed`，重啟 DB，啟動全新 MCP/status process | run/status public projection digest 相等；Codex/test/Git/Graphify/Hermes/Memory counters與 snapshot/staging/row footprints完全不變 |
| F07 | 將 project B、舊 commit/snapshot、錯誤 receipt/head/spec/fence 代入 project A status/reconciliation | 每一種 substitution 都在 effect 前拒絕；project A/B durable rows 與 artifacts 無 mutation/leak |
| F08 | 對成功與每個 failure path 執行 finally cleanup | installed PostgreSQL service snapshot 不變；marker child/listener/DB/worktree/credential copy/temp/snapshot/staging/Hermes roots 符合 exact allowlist；unknown ownership 保留並 fail closed |

## Required Machine Evidence

目前 runner 只有在八個 scenario 全部通過時，才可輸出：

```text
TASK044_COMPONENT_FAULT_RECOVERY_GATE=PASS
SCENARIOS=F01,F02,F03,F04,F05,F06,F07,F08
FRESH_STATUS_ZERO_RERUN=PASS
CLEANUP_FOOTPRINT=PASS
```

marker-owned evidence bundle 必須是 bounded JSON，至少包含：

- `schema_version = lattice.task044.component-fault-recovery.v1`；
- base 與 TASK-038 stable checkpoint SHA；
- run/scenario/component/stage/injection ID；
- pre-fault intent/dispatch receipt digest；
- typed failure certainty 與 reconciliation receipt digest；
- PostgreSQL identity、task/memory/current-head digest；
- process/database restart count；
- Codex/verification/Git/Graphify/Hermes/Memory effect counters before/after
  fresh Status；
- run/status projection digests；
- exact cleanup allowlist、observed paths/processes/listeners 與 unexpected
  footprint（必須為空）。

不得保存 raw prompt、source、diff、SQL、DSN、credential/token、provider response、
child stderr、URL 或任意未界定錯誤文字。

## Current Execution

本 branch 已同步到 current combined checkpoint `65f2902`，且保留 TASK-038 stable
checkpoint `2b424ec` 作為追溯基礎。唯一整合入口為：

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\scripts\run-task044-component-fault-recovery-gate.ps1
```

runner 先拒絕 dirty worktree、錯誤 branch、缺少 base ancestry、skip、缺少固定
marker、任一 command 非零、F01-F08 coverage 不完整，以及驗證後 source/HEAD 漂移。
它只把 current owner tests、Task076 current PostgreSQL profile、Task068 Hermes replay、
TASK-044 durable cancellation 與 TASK-050 fresh-status acceptance 彙整成 bounded JSON；
不保存 raw prompt、source、SQL、credential 或 child output。

## Acceptance Criteria

- [x] 盤點 base `845328d` 與受保護 TASK-038 dirty snapshot 的 branch/worktree/status、
  AGENTS/PLANS/HANDOFF、SPEC/ADR/constitution/ticket 與相關 harness/test evidence。
- [x] 將現有 evidence 分成 `ME`、`DO`、`MI`、`BL`，並記錄真正缺口。
- [x] 鎖定 F01-F08 bounded scenarios、zero-rerun counters、restart evidence 與 cleanup
  allowlist。
- [x] 保護 TASK-043 pure crate/ticket 與 One Gateway / One Truth / One Writer boundaries。
- [x] TASK-038 clean stable checkpoint 已提供、驗證並同步進本 branch。
- [x] 依同步後 contracts 修訂 versioned allowed paths 與 fault-fixture/runner contract。
- [x] 輕量 RED contract tests 先證明 scenario schema、first-failure suppression、
  reconciliation/zero-rerun counters 與 cleanup manifest fail closed。
- [x] marker-owned disposable live gate 逐一通過 F01-F08；不使用 production service、
  paid/provider、public listener、user repository 或非 marker resource。
- [x] fresh process/status equality、zero external rerun 與 exact cleanup 全部有 current
  machine evidence。
- [x] focused verification、scope/diff/secret checks 與必要 review 通過；full workspace
  matrix 仍依當時資源與 owner gate 分開執行，不得由本 ticket 偷跑。

## Non-Goals

- 修改 TASK-038/TASK-043、pure Codebase Memory、root plan/handoff 或既有 worktree。
- 建立第二套 Orchestrator、Writer Lease、PostgreSQL migration/profile、task owner、
  Memory truth、status cache 或 recovery daemon。
- 任意 retry unknown effect、把 process exit 0 當 tool success、把 fake/scripted
  fixture 當 production/provider acceptance。
- 啟動或修改 installed PostgreSQL service、WSL distribution、production DB/service、
  credential/role、public network、paid provider、user project。
- push、merge、deploy、release、primary-branch change 或 permanent deletion。

## Current Handoff

`DONE_LOCAL / NOT_INTEGRATED`

exact implementation source `f6e66da` 已通過 runtime、live PostgreSQL
initial/restart、Task068 Hermes replay、Task050 fresh Status、strict Clippy、格式、
repository check、read-only code/architecture review 與完整 F01-F08 aggregate。
bounded JSON evidence 位於 ignored `target/task044-component-fault-recovery/` 下，run id
為 `e5784371df1a46c78a64b8f1cf9c59cd`。本 branch 尚未 push/merge；deploy 或 release
亦未執行或授權。
