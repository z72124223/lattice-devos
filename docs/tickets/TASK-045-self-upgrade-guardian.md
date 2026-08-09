---
ticket_id: TASK-045
spec_id: SPEC-002
spec_version: 27
module_id: self-upgrade-guardian
constitution_version: 1.0
constitution_source: docs/modules/V2_AMENDMENT_PROPOSAL.md
status: blocked
parallel_safe: false
depends_on:
  - TASK-015
  - TASK-016
  - TASK-022
  - TASK-038
  - TASK-043
allowed_paths:
  - docs/tickets/TASK-045-self-upgrade-guardian.md
  - contract-checkpoints/task-045-self-upgrade-guardian/**
likely_files: []
branch: feature/task-045-047-guardian-contracts-20260810
base_commit: 845328dcc06d51c7554c93a09739a27ddd827941
---

# TASK-045：獨立 Self-Upgrade Guardian 契約關卡

## Objective

先凍結 MVP-3 `self-upgrade-guardian` 1.0 的可執行純契約與 RED
fixtures；等前置 owner 與 TASK-038 形成正式 stable checkpoint 後，才重新同步並另行
擴充 implementation `allowed_paths`。本 checkpoint 不建立 Guardian credential、角色、
service 或 PostgreSQL procedure，不改 active slot，不停止正式服務，也不執行
activation、deployment、release。

Guardian 必須是獨立、受保護且由作業系統驗證身分的管理 surface。Normal
Gateway、OpenClaw、GPT、候選 binary、active daemon 與一般 task IPC 均不能取得或
轉授 protected claim authority。

## Current Dependency Evidence

| 前置 owner／ticket | 基準 `845328d` 與 live 證據 | 本 ticket 判定 |
|---|---|---|
| Approval Verifier / TASK-015 | pure/fake subject、nonce、expiry、replay/current-head 已有；OS trust root、live crypto、protected atomic claim 未通過 | `BLOCKED` |
| Project Registry / TASK-022 | pure owner 已有；SPEC-002 AC-36 與 current durable readiness 尚未形成可依賴 checkpoint | `BLOCKED` |
| Writer Lease / TASK-014 + TASK-038 | pure lease 已有；TASK-038 的 real PostgreSQL lease/fence、restart、unknown-outcome acceptance 仍 `in-progress` | `BLOCKED` |
| Artifact Store / TASK-016 | pure/fake owner 已有；SPEC-002 AC-19 durable metadata/filesystem lifecycle 仍 open | `BLOCKED` |
| TASK-038 | Task Submit/Status、controlled Codex、fresh-process durable status 尚未 PASS | `BLOCKED` |
| TASK-043 / Step 11 | 現有工作樹有未提交變更，沒有可同步 stable checkpoint | `BLOCKED` |
| `self-upgrade-guardian` constitution | 已批准的 1.0 內容只存在 `V2_AMENDMENT_PROPOSAL.md`；獨立 active constitution 尚不存在 | `BLOCKED` |

任何一列未 ready 時，最多保留本 ticket 與 executable RED fixture，不得以 fake
adapter、文件存在或 caller Boolean 模擬 authority。

## Protected Surface Contract

1. Protected approval 只能由 Guardian-owned OS-authenticated local surface 建立；
   normal Gateway 只能 initiate/display request，不能滿足 claim。
2. Approval Verifier 只驗證完整 subject、identity、trust lane、expiry 與 proof；它不
   消耗 protected nonce。
3. Guardian-only `claim_activation` 是唯一 nonce consumer；同一 PostgreSQL transaction
   必須鎖定 release/nonce、重驗 receipt/subject/database time、標記 nonce used、append
   `ACTIVATION_CLAIMED`，並把 runtime admission 改為 `DRAINING`。
4. Guardian 只能寫 dedicated release/daemon-epoch procedures；不能寫 task、project、
   registry、lease、artifact、memory、review、capability 或一般 approval truth。
5. A/B slot 與 manifest 均 immutable，source/target slot 必須不同；active bundle 絕不
   in-place overwrite。
6. 第一版 A/B activation 的 migration set 必須為空，且 schema compatibility 必須對
   active、candidate、rollback 三者都成立。

## Activation State Machine

| Current | Event / proof | Next | Admission after commit | Fail-closed rule |
|---|---|---|---|---|
| `STAGED` | immutable manifest、distinct inactive slot、read-only shadow PASS | `PROMOTION_APPROVED` | `ACTIVE` | shadow 不得寫 control stream 或取得 product lease |
| `PROMOTION_APPROVED` | Guardian-only atomic claim | `ACTIVATION_CLAIMED` | `DRAINING` | nonce consumption、claim event、admission change 必須同 transaction |
| `ACTIVATION_CLAIMED` | zero lease/effect/unknown outcome/writable Codex child proof | `OLD_DAEMON_DRAINED` | `DRAINING` | 任一未知或仍 active 就停留，不可切 slot |
| `OLD_DAEMON_DRAINED` | append pointer-write intent，再寫 checksum-bound projection | `SLOT_POINTER_WRITTEN` | `DRAINING` | PostgreSQL 是 truth；file pointer 只是 projection |
| `SLOT_POINTER_WRITTEN` | immutable candidate digest recheck，read-only start | `CANDIDATE_STARTED` | `DRAINING` | candidate 尚無 writer admission |
| `CANDIDATE_STARTED` | atomic higher non-wrapping epoch assignment | `EPOCH_ACTIVATED` | `CANARY` | old instance/epoch 即使保留連線也必須被 DB 拒絕 |
| `EPOCH_ACTIVATED` | reserved system-stream append + replay 與 health window PASS | `HEALTH_WINDOW_PASSED` | `CANARY` | user-project 與非 canary mutation 全拒絕 |
| `HEALTH_WINDOW_PASSED` | exact finalize transaction | `ACTIVATION_FINALIZED` | `ACTIVE` | 只有 finalize 可進入 `ACTIVE` |
| 任一 claimed 後失敗 | append rollback request | `ROLLBACK_REQUESTED` | `DRAINING` 或 `CANARY` | 不得直接跳回 source slot |
| `ROLLBACK_REQUESTED` | verified compatible prior-slot pointer intent/outcome | `ROLLBACK_SLOT_WRITTEN` | 非 `ACTIVE` | prior slot digest/compatibility 不明即 reconciliation |
| `ROLLBACK_SLOT_WRITTEN` | prior binary 以新 instance read-only start | `ROLLBACK_DAEMON_STARTED` | 非 `ACTIVE` | 不復用舊 instance |
| `ROLLBACK_DAEMON_STARTED` | assign epoch `> failed_activation_epoch` | `ROLLBACK_EPOCH_ACTIVATED` | `CANARY` | epoch 不可下降、相等、wrap 或重用 |
| `ROLLBACK_EPOCH_ACTIVATED` | same reserved write-canary/health PASS + finalize | `ROLLED_BACK` | `ACTIVE` | rollback 也不能略過 canary |
| 任一 ambiguous state | owner-produced disagreement/unknown evidence | `RECONCILIATION_REQUIRED` | `RECONCILIATION_REQUIRED` | 不啟動可寫 daemon，不接受 effect |

Exact same-command retry 可回傳相同 terminal receipt；changed content、skipped state、
stale expected state、duplicate first effect 或 epoch mismatch 必須 deterministic deny。

## Acceptance Criteria

- [x] Phase A ticket、dependency graph、protected-surface boundary 與 state machine 已凍結。
- [x] Phase A fixture 能由獨立 deterministic contract checkpoint 解析，且不讀 DB、
      filesystem state、clock、environment、network 或 credential。
- [x] Acceptance feature 目前產生可重現非零 RED，錯誤只列出缺少的 owner behavior，
      不宣稱 activation authority。
- [ ] 獨立 active `self-upgrade-guardian` 1.0 constitution 已在 stable checkpoint 存在並
      通過治理驗證。
- [ ] Approval/Registry/Lease/Artifact/TASK-038/TASK-043 所有上表前置均有 current
      executable readiness evidence。
- [ ] 重新同步 stable base 後，另行擴充 implementation allowlist，完成 pure Guardian
      planner 與 narrow ports 的逐項 RED/GREEN。
- [ ] protected OS identity/ACL/trust root 與 Guardian-only atomic claim 由 disposable
      live gate 直接驗證；pure/fake 結果不能勾選此項。
- [ ] activation、drain、canary、reconciliation、rollback 實作完成前，不啟用任何真實
      service 或 slot。

## Non-Goals

真實 credential/role/service、production DB mutation、schema migration、active-slot
replacement、正式服務 stop/restart、normal Gateway protected authority、第二套
orchestrator/domain owner/truth/writer、push、merge、deploy、release。

## Module And Constitution Constraints

- Governing source：`docs/modules/V2_AMENDMENT_PROPOSAL.md` 中已批准的
  `self-upgrade-guardian 1.0`；它明載每個 implemented module 仍需獨立 active
  constitution。
- 相容：ADR-007、ADR-009、ADR-013、SPEC-002 v27；本 ticket 不弱化其 nonce、epoch、
  admission、first-MVP-no-migration 與 protected-surface 要求。
- 不相容／缺口：獨立 constitution 與 live narrow procedure owner 尚未 ready，因此
  implementation activation 被硬阻擋。

## Dependencies And Overlap

`parallel_safe: false`。TASK-038 正修改 Contracts、Ports、Writer Lease、Postgres Store、
Orchestrator、runtime 與 governance root；TASK-045 不得複製或平行修改那些 dirty
paths。本 Phase A allowlist 只有自己的 ticket 與獨立 checkpoint root。等正式 stable
checkpoint 後必須重新同步並重新盤點 paths，不能直接把 Phase A branch 當
implementation base。

## TDD Behaviors

1. Protected release 不能由 normal gateway/OS-user lane 構造。
2. Closed state/event/admission names 與 exact legal transition matrix。
3. Distinct immutable A/B slots、complete manifest、empty migration set。
4. Atomic nonce/claim/`DRAINING` contract 與 concurrent exact retry。
5. Complete drain proof；unknown lease/effect/process 失敗關閉。
6. Higher daemon epoch、stale live-connection denial、`CANARY` admission。
7. Reserved system-stream write canary、health disagreement、finalize-only `ACTIVE`。
8. PostgreSQL-wins boot-pointer reconciliation 與 higher-epoch rollback。

Phase A 只完成 fixture 與 RED checkpoint；依 `implement-tdd`，因 preconditions blocked，
不進入 GREEN／implementation。

## Verification

| Check | Command | Expected evidence |
|---|---|---|
| Fixture structure | `CARGO_BUILD_JOBS=2 cargo test --manifest-path contract-checkpoints/task-045-self-upgrade-guardian/Cargo.toml --locked` | zero exit；closed rows/IDs/state/admission 與既有 normal-gateway denial guardrail 通過 |
| Intentional RED | `CARGO_BUILD_JOBS=2 cargo test --manifest-path contract-checkpoints/task-045-self-upgrade-guardian/Cargo.toml --locked --features acceptance` | non-zero；列出尚非 `READY` 的 Guardian requirements |
| Scope | `git diff --check` 與 changed-path allowlist | 僅本 ticket 與本 checkpoint root |

## Human Gate

目前無需額外人工作業即可保存 Phase A checkpoint。建立/修改 credential、role、service、
trust root、database schema/procedure、啟動 Guardian、切換 slot、stop production、merge、
deploy 或 release 都是未授權且明確阻擋的後續 action。
