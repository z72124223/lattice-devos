---
ticket_id: TASK-047
spec_id: SPEC-002
spec_version: 27
module_id: self-upgrade-guardian
constitution_version: 1.0
constitution_source: docs/modules/self-upgrade-guardian/MODULE_CONSTITUTION.md
status: blocked
parallel_safe: false
depends_on:
  - TASK-046
allowed_paths:
  - docs/tickets/TASK-047-final-integration-readiness.md
  - contract-checkpoints/task-047-final-readiness/**
likely_files: []
branch: feature/task-045-047-guardian-contracts-20260810
base_commit: 845328dcc06d51c7554c93a09739a27ddd827941
---

# TASK-047：Final Integration / Readiness Evidence

## Objective

只定義 Step 14 的 combined verification、independent review、CI、merge authorization
與 user acceptance evidence contract。TASK-047 不實作 product behavior、不重新執行
TASK-045/046 activation、不 merge、不 deploy、不 release，也不因 local tests 或文件齊全
而宣稱 readiness。

## Dependency And Readiness Flow

```text
TASK-045 implementation + protected live gates READY
  -> TASK-046 every disposable drill PASS
  -> TASK-047 evidence packet complete
  -> independent review PASS
  -> commit-bound CI PASS and merge authorization present
  -> responsible-user acceptance present
  -> READY_FOR_AUTHORIZED_MERGE
```

任何一步 `BLOCKED`、`MISSING`、`STALE`、`FAILED`、`UNKNOWN` 或只靠文件陳述時，整體
結果都是 `BLOCKED`。`READY_FOR_AUTHORIZED_MERGE` 不是 merge、deployment 或 release。

## Evidence Matrix

| Gate ID | Required immutable binding | Current Phase A status | Completion rule |
|---|---|---|---|
| `E01_BASE` | exact base/head/tree、branch、worktree、clean/known diff | `PASS_PHASE_A_ONLY` | implementation sync 後重取 current evidence |
| `E02_SCOPE` | TASK-045/046/047 allowed paths 與完整 diff | `PASS_PHASE_A_ONLY` | changed path 零 drift |
| `E03_DEPENDENCIES` | Approval/Registry/Lease/Artifact/TASK-038/TASK-043 receipts/checkpoints | `BLOCKED` | 每一 owner 都 current、live/fake 標籤正確 |
| `E04_GUARDIAN_CONTRACT` | TASK-045 focused pure + live protected claim/epoch/admission evidence | `BLOCKED` | pure 與 live gates 分開 PASS |
| `E05_AB_DRILLS` | TASK-046 F01-F23 durable drill receipts | `BLOCKED` | 每 row PASS，沒有 skipped/waived ambiguity |
| `E06_FOCUSED_TESTS` | affected package/test command + exact commit | `BLOCKED` | zero exit；本 Phase A 不跑 full workspace matrix |
| `E07_FULL_LOCAL` | format、strict lint、workspace/integration matrix | `BLOCKED` | 只在重負載槽批准後執行並保留 current output |
| `E08_CODE_REVIEW` | independent reviewer identity、commit/tree、findings | `MISSING` | 所有 actionable finding closed |
| `E09_SECURITY_ARCH` | independent security/architecture review、contract versions | `MISSING` | zero unresolved blocker；無自我批准 |
| `E10_CI` | remote CI provider、commit SHA、required checks、result/time | `MISSING` | commit-bound PASS；local config 不算 remote CI |
| `E11_SYNC_CONFLICT` | target head、merge base、fresh conflict/readiness analysis | `MISSING` | current、無 conflict、無 hidden drift |
| `E12_MERGE_AUTH` | responsible-user exact merge subject authorization | `MISSING` | 明確且未過期；不得由 model/Guardian 推導 |
| `E13_USER_ACCEPTANCE` | exact candidate/evidence packet 的 user acceptance | `MISSING` | 明確 responsible-user acceptance |
| `E14_RELEASE_AUTH` | protected release subject/nonce/target slot/epoch | `OUT_OF_SCOPE` | 本 ticket 永不執行；另立 protected action |
| `E15_DEPLOYMENT` | target/environment/release authority | `OUT_OF_SCOPE` | 本 ticket 永不執行 |

## Readiness State Machine

| Current | Required event | Next | Prohibited shortcut |
|---|---|---|---|
| `BLOCKED` | TASK-045 + TASK-046 全部 current PASS | `EVIDENCE_COLLECTING` | 不能用 fixture/文件代替 drills |
| `EVIDENCE_COLLECTING` | E01-E11 全部 current PASS | `EVIDENCE_COMPLETE` | local PASS 不代替 CI/review |
| `EVIDENCE_COMPLETE` | E12 exact merge authorization | `READY_FOR_USER_ACCEPTANCE` | Guardian/model 不授權 merge |
| `READY_FOR_USER_ACCEPTANCE` | E13 responsible-user acceptance | `READY_FOR_AUTHORIZED_MERGE` | silence 不算 acceptance |

`MERGED`、`DEPLOYED`、`RELEASED` 不屬於本 state machine；即使 readiness 成立也不自動
執行任何一項。

## Acceptance Criteria

- [x] Phase A evidence matrix、readiness state machine、CI/merge/user-acceptance 分離完成。
- [x] Pure fixture test 證明 current packet 必須為 `BLOCKED`，並拒絕把
      `OUT_OF_SCOPE` 當 PASS。
- [x] Acceptance feature 目前 deterministic RED，會列出 E03-E13 的缺證而不是輸出
      readiness。
- [ ] TASK-046 全部 disposable drills 有 current durable PASS evidence。
- [ ] Focused/full checks、independent code/security/architecture review、remote CI、sync/
      conflict analysis 各自綁定相同 candidate commit/tree 並 current PASS。
- [ ] Exact merge authorization 與 responsible-user acceptance 分別存在。
- [ ] 結果最多標 `READY_FOR_AUTHORIZED_MERGE`；本 ticket 沒有 merge/deploy/release
      side effect。

## Non-Goals

新增 implementation、補做 activation、建立 credential/service/role、production DB、
active-slot replacement、merge、deploy、release、push、把 local CI config 當遠端 required
check、把 reviewer/model/Guardian 當 responsible-user acceptance。

## Dependencies And Overlap

`parallel_safe: false`。TASK-047 必須在 TASK-046 完成後才收集 final evidence；所有
evidence 必須綁同一 immutable candidate。Phase A 只有自己 ticket/checkpoint root，與
TASK-045/046 以及所有受保護工作樹不重疊。

## Verification

| Check | Command | Expected evidence |
|---|---|---|
| Fixture structure | `CARGO_BUILD_JOBS=2 cargo test --manifest-path contract-checkpoints/task-047-final-readiness/Cargo.toml --locked` | zero exit；E01-E15 唯一且 current result 為 `BLOCKED` |
| Intentional RED | `CARGO_BUILD_JOBS=2 cargo test --manifest-path contract-checkpoints/task-047-final-readiness/Cargo.toml --locked --features acceptance` | non-zero；缺少的 E03-E13 清楚列出 |
| Heavy/full matrix | `BLOCKED` | 依資源更新，ready 後才申請重負載槽 |

## Human Gate

本 ticket 的 Phase A 文件/fixture 不需要新 gate。Merge authorization、user acceptance、
protected release、deployment 與 release 都是分離的人類／受保護關卡；目前均未授權。
