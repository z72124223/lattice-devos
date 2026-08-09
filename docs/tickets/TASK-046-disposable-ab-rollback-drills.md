---
ticket_id: TASK-046
spec_id: SPEC-002
spec_version: 27
module_id: self-upgrade-guardian
constitution_version: 1.0
constitution_source: docs/modules/V2_AMENDMENT_PROPOSAL.md
status: blocked
parallel_safe: false
depends_on:
  - TASK-045
allowed_paths:
  - docs/tickets/TASK-046-disposable-ab-rollback-drills.md
  - contract-checkpoints/task-046-ab-rollback-drills/**
likely_files: []
branch: feature/task-045-047-guardian-contracts-20260810
base_commit: 845328dcc06d51c7554c93a09739a27ddd827941
---

# TASK-046：Disposable A/B Rollback Drills

## Objective

在 TASK-045 完整實作並通過其所有 authority/atomicity gates 後，才於完全 disposable
process、filesystem、database 與 slot fixture 執行 A/B、power-loss、canary、health
disagreement、reconciliation 與 higher-epoch rollback drills。本 Phase A 只凍結 fault
matrix 與 executable RED fixture，不啟動 service/DB/WSL，也不執行任何 drill。

## Dependency Graph

```text
TASK-045 READY
  -> immutable Guardian candidate + narrow protected claim contract
  -> TASK-046 disposable fault drills
  -> TASK-047 combined final readiness evidence
```

目前 TASK-045 為 `BLOCKED`，所以 TASK-046 不得 activation、不得以 stub/fake authority
跑出 PASS，也不得把 fixture parser 成功解讀為 rollback readiness。

## Fault Matrix

| ID | Injection / disagreement | Required durable observation | Required disposition | Forbidden claim/effect |
|---|---|---|---|---|
| `F01_MANIFEST_CORRUPT` | manifest/binary/lock/evidence digest mismatch | claim 前零 release mutation | `DENY_STAGED` | 不 consume nonce、不 drain |
| `F02_SLOT_ALIAS` | source 與 target 指向同 slot/bundle | invalid immutable slot binding | `DENY_STAGED` | 不 overwrite active bundle |
| `F03_SCHEMA_MIGRATION` | migration set 非空或 schema incompatible | first-MVP compatibility denial | `DENY_STAGED` | 不執行任何 DDL/migration |
| `F04_NORMAL_GATEWAY_CLAIM` | OpenClaw/GPT/task token 嘗試 protected claim | wrong lane/identity denial | `DENY_APPROVAL` | normal Gateway 不升權 |
| `F05_APPROVAL_EXPIRED` | DB time 等於或晚於 expiry | nonce 保持未 claim 或 terminal unavailable | `DENY_APPROVAL` | 不進 `DRAINING` |
| `F06_APPROVAL_REPLAY` | 同 nonce 被第二 activation 使用 | one-use binding mismatch | `DENY_APPROVAL` | 不產生第二 claim |
| `F07_CONCURRENT_CLAIM` | 兩個 Guardian request 同時 claim | exactly one atomic winner；exact retry 同 receipt | `CONVERGE_CLAIMED` | 不雙 consume/雙 drain |
| `F08_CLAIM_ACK_LOSS` | commit 後 response 遺失 | reconnect 查 durable nonce/event/admission | `REPLAY_OR_RECONCILE` | 不盲目重做 claim |
| `F09_DRAIN_ACTIVE_LEASE` | valid writer lease 尚在 | state 保持 `ACTIVATION_CLAIMED`/`DRAINING` | `WAIT_DRAIN` | 不切 slot |
| `F10_DRAIN_UNKNOWN_EFFECT` | claimed/running/unknown effect 存在 | unknown effect receipt 可追蹤 | `WAIT_RECONCILE` | 不假設 effect 未發生 |
| `F11_DRAIN_WRITABLE_CHILD` | writable Codex process tree 仍活著或未知 | termination proof 缺失 | `WAIT_DRAIN` | 不宣稱 old daemon drained |
| `F12_POWER_BEFORE_POINTER` | pointer intent 後、file write 前斷電 | PostgreSQL state precedes pointer outcome | `RESTORE_FINALIZED_SLOT` | file 不成 truth |
| `F13_POWER_AFTER_POINTER` | file write 後、outcome 前斷電 | DB 已/未確認 write 的 exact split | `REWRITE_OR_RECONCILE` | 不猜測 active slot |
| `F14_POINTER_CHECKSUM` | boot projection checksum/manifest/epoch 不一致 | disagreement evidence | `RECONCILIATION_REQUIRED` | 不啟動 writer |
| `F15_CANDIDATE_CRASH_LOOP` | candidate start/crash 超過固定 threshold | crash samples + timeout | `ROLLBACK_REQUESTED` | 不放寬 health contract |
| `F16_STALE_CONNECTION` | old daemon 用既有 DB connection 寫入 | instance/epoch/admission same-tx denial | `DENY_OLD_WRITER` | 不接受 stale mutation |
| `F17_CANARY_USER_WRITE` | candidate 在 `CANARY` 寫 user project | exact repository admission denial | `DENY_CANARY_WRITE` | 只有 reserved health stream 可寫 |
| `F18_CANARY_REPLAY_MISMATCH` | reserved append/replay 不同或遺失 | canary receipt disagreement | `ROLLBACK_REQUESTED` | 不 finalize |
| `F19_HEALTH_DISAGREEMENT` | candidate 與 Guardian health 結果不一致 | immutable Guardian minimum contract wins | `ROLLBACK_OR_RECONCILE` | candidate 不得降低 threshold |
| `F20_ROLLBACK_EPOCH` | rollback epoch `<=` failed activation epoch | epoch comparison denial | `DENY_ROLLBACK_START` | 不 reuse/decrement epoch |
| `F21_ROLLBACK_SLOT_BAD` | prior slot digest/schema incompatible | verification disagreement | `RECONCILIATION_REQUIRED` | 不啟動未證明 binary |
| `F22_POWER_EACH_EDGE` | 每個 durable transition 的 intent/outcome 間斷電 | restart 後唯一 state 或明確 ambiguity | `RESUME_ROLLBACK_OR_RECONCILE` | 不跳 state、不重做外部 effect |
| `F23_GUARDIAN_UNAVAILABLE` | claim 後 Guardian/service 不可用 | DB 保留 saga/admission；recovery status 可讀 | `STOPPED_OR_RECONCILE` | normal Gateway 不接管 protected writer |

## Acceptance Criteria

- [x] Phase A fault matrix 覆蓋 manifest、approval、atomic claim、drain、pointer、process、
      epoch、canary、health、power-loss、reconciliation、rollback 與 Guardian unavailable。
- [x] Fixture schema 與 required coverage 可由獨立 pure test 驗證。
- [x] Acceptance feature 目前 deterministic RED，所有 drill 皆標 `BLOCKED_TASK045` 或
      `RED_NOT_RUN`。
- [ ] TASK-045 在重新同步後具備 current GREEN implementation、protected live gate 與
      disposable-only activation authority。
- [ ] 每一 fault row 均在 owned disposable roots 實際注入並取得 durable before/after
      evidence；單元測試或 mock 不替代 process/power-loss drill。
- [ ] A 與 B bundle 維持 immutable；每輪 cleanup 先驗證 exact disposable root，且不
      接觸 active/production slot。
- [ ] Rollback binary 以新 instance、strictly higher epoch 進 `CANARY`，通過相同 write
      canary 後才 finalize `ACTIVE`。

## Non-Goals

正式服務 stop/restart、active-slot promotion、production DB、使用者 repository、真實
credential/role/service、schema migration、public exposure、merge、deploy、release。

## Dependencies And Overlap

`parallel_safe: false`。本 ticket 只可在 TASK-045 `READY` 後執行，而且 drills 會共享
Guardian state machine 與 immutable slot contract。本 Phase A path 與 TASK-045、
TASK-038、TASK-042、TASK-043 完全分離；未同步前不新增 process/DB/WSL harness。

## TDD Behaviors

1. 先由 fixture coverage test 鎖定 F01-F23、expected disposition 與禁止效果。
2. 每次只實作一個 disposable fault injection，先取得非零 RED，再補最小 recovery。
3. 同一 edge 的 before-effect、after-effect、commit-ack-loss 分開證明。
4. 依 state machine 順序執行；未通過前一 edge 不執行後一 activation edge。

Phase A 不進入上述實作循環，只保存 RED matrix。

## Verification

| Check | Command | Expected evidence |
|---|---|---|
| Fixture structure | `CARGO_BUILD_JOBS=2 cargo test --manifest-path contract-checkpoints/task-046-ab-rollback-drills/Cargo.toml --locked` | zero exit；F01-F23 無重複且 required dispositions 完整 |
| Intentional RED | `CARGO_BUILD_JOBS=2 cargo test --manifest-path contract-checkpoints/task-046-ab-rollback-drills/Cargo.toml --locked --features acceptance` | non-zero；沒有 drill 被誤標 PASS |
| Future drills | `BLOCKED`；TASK-045 READY 後另定 bounded command | 目前不得執行 service/DB/WSL/process/filesystem activation |

## Human Gate

Phase A fixture 不需要額外 gate。任何真實 service、credential、role、production DB、
active slot、正式 stop/restart、deployment 或 release 仍需新的明確 authority；本 ticket
文字不能授權那些動作。
