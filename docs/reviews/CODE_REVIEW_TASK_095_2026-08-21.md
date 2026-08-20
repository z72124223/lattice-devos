# TASK-095 Diagnostic Review — 2026-08-21

## Review result

`DIAGNOSTIC_EVIDENCE_COMPLETE; UNDERLYING_LEAF_UNKNOWN; TASK-033_IN_PROGRESS`。
本 review 只審查已完成的單次 runtime diagnostic 與 read-only trace，不是 TASK-033
成功驗收，也不是產品碼修復審查。

## Scope and provenance

- branch `feature/task-095-runtime-root-cause-acceptance`，source
  `ef1c3741a862493a7edeea815ef5a7a101aecfcd`。
- fixture `target/lattice-delivery/9c205fa8acc54aa2881fdba51cb6d68d`；舊 fixture
  `dd8f708cf0ac4721b12575ce12f44a1a` 僅讀取對照。
- wrapper 一次 exit 1；child bounded diagnostic exit 2；artifact 固定於
  `evidence/runtime-run-failure-diagnostic.json`。只記錄 bytes、flags、error code，
  不記錄 raw stdout/stderr 或秘密。

## Narrow call graph

`lattice-codex-adapter` identity/process error → orchestrator terminal Failed receipt
→ `apps/lattice-runtime/src/composition.rs::map_orchestrator_error`（以 Failed
receipt 改映射 `LatticedErrorKind::DeliveryFailed`）→ `RuntimeError::Latticed` →
`apps/lattice-runtime/src/main.rs` JSON `code` → exit 2 → TASK-093 wrapper bounded
artifact。已確認的最窄缺口是 composition-to-main 的 stage/cause-code collapse。

## Findings

Confirmed：新 fixture 沒有 identity success 必需的 `schema/`，也沒有 scripted
server 成功後應寫出的 `delivery/repo/answer.txt`；nested PG/process/listener
teardown 完成且 ownership 無歧義；沒有 wrapper 未收集的 run-owned logs/artifacts。

Bounded hypotheses：identity preflight 的 schema child/containment/version/deadline
leaf，或較低機率 workspace prepare。每一項都只能以不含 payload 的 identity-leaf
matrix、receipt mapping preservation、scripted schema/answer protocol test 離線
證偽；不得把 hypothesis 寫成 root cause。

Excluded：5432、舊 fixture、TASK-094 資源、TASK-033 ticket/product code、再次
live、ReconciliationRequired-only ambiguous branches，均未觸碰。

## Restart wording and next action

Initial/restart checks for TASK019 completed before DeliveryRun. Delivery restart/status
replay did not run because DeliveryRun failed first；因此只能說「未發生」，不能說
「通過」。TASK-096 應在上述 runtime owned paths 保留 bounded stage + cause code，
並加入無秘密 focused tests；本 review 不修改它。

## Verification record

先前 focused evidence：`npm.cmd run verify` 48/48、PowerShell AST PASS、diff check
PASS。此文件變更須再執行 project check/verify、diff/secret scan；任何 finisher 或
dashboard 缺失都須 fail-closed，不以手動 push 代替。
