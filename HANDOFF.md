# HANDOFF

## Status

`DONE`

## Objective and scope

- Objective: 讓進行中的父工作可耐久 `BLOCKED` 於一個有界依賴，安全建立
  子 branch/worktree，整合後解除阻塞，並由 PostgreSQL 在新程序還原完整
  dependency binding 與 next action。
- In scope: Foreman snapshot projection、MCP 契約、Git guard、bounded CLI、
  PostgreSQL replay、測試／審查、GitHub merge、安裝收據與無 App restart
  的 fresh Runtime 驗證。
- Out of scope: 公開網路、付款、帳戶／憑證變更、安全控制降低、未知髒
  工作樹整理，以及未來產品需求。

## Completed work

- 結構化 `lattice.dependency-blocker/1.0` 保存父／子 task、子 branch/worktree、
  base SHA 與 `COMPLETE_DEPENDENCY`；舊 blocker 保持 opaque replay。
- `BLOCKED` 與 `BLOCKED -> ACTIVE` 都受 marker、Git identity、cleanliness、
  ancestry 與 integration proof 保護；直接 `COMPLETED`、衝突或不確定狀態
  fail closed。
- Node CLI 只建立規則化的 dependency branch/worktree；Runtime status 1.1
  投影 `depends_on`、狀態、verification 與 next action。
- PostgreSQL live fixture 證明 exact retry、阻塞、實際子工作樹、整合、續接、
  fresh-process replay、競爭與失敗分類。

## Files changed

| Path | Why | Verification |
|---|---|---|
| `crates/lattice-foreman-state/**` | binding、commitment、replay state machine | 16/16、strict Clippy |
| `apps/lattice-runtime/**` | MCP、Git guard、status 1.1、live fixture | Runtime/MCP/full live PASS |
| `scripts/lattice-dependency-worktree.mjs`、`test/git-workspace.integration.test.js` | bounded child worktree | Node 117 pass、13 Git cases |
| `crates/lattice-ports/**`、`crates/lattice-postgres-store/**` | versioned projection boundary | full crate tests、strict Clippy |
| `docs/specs`、`docs/tickets`、`docs/modules`、`docs/reviews` | frozen contracts and evidence | `npm run check` |

## Workflow ledger

| Stage | Status | Evidence / artifact |
|---|---|---|
| Local implementation/tests | DONE | `2017a6b`, `e45bb71`, `805e4ce`; Node/Rust suites |
| Independent review | DONE | final code/architecture GO; no P0-P3 |
| Integration/CI/merge | DONE | PR #23, CI `verify` PASS, merge `6dc1e303` |
| Install/reload | DONE | artifact SHA `a7a1f74c…`; receipt `5421ff01…`; fresh Codex reload |
| Durable replay | DONE | live runs `a39cff…` and post-deploy `b6d5f5…` |

## Verification

- Commands and exit codes: `npm.cmd run verify` 0; required Cargo test suites 0;
  `cargo fmt --check` 0; `git diff --check` 0; scoped strict Clippy 0.
- Tests/build/lint: Control 17/17; Node 117 pass/0 fail/1 platform skip;
  Runtime library 134 pass/0 fail/2 coordinated-live ignored; MCP 37/37;
  Foreman 16/16; Ports/Store/Orchestrator full suites PASS. Full Runtime strict
  Clippy retains the same 22 pre-existing diagnostics on product and feature,
  with zero TASK-106 symbol lines.
- CI: GitHub `verify` PASS in 46 seconds on PR #23 head `805e4ce`.
- Runtime: installed merge artifact exposed exactly seven tools and Runtime
  projection 1.1; generation 3 replay was `VERIFIED` with `CONTINUE`.

## Review and integration

- Code review: independent final GO, P0-P3 = 0.
- Architecture review: independent final GO; PostgreSQL remains sole durable truth,
  seven tools and existing dependency directions preserved.
- Branch/worktree synchronization: isolated integration clean; product worktree,
  remote default branch and merge commit matched `6dc1e303` before this evidence-only
  finalization.
- Merge status and authorization: implementation PR #23 merged under explicit
  user authorization. No branch protection/ruleset/required review exists, so no
  service enforcement is claimed.

## Risks and open decisions

- Existing desktop-connected MCP processes continue using their already-open old
  binary. Four active Codex writer locks correctly prevented persistent global
  pointer rotation. The installed version was instead verified through fresh
  Codex reload and a fresh MCP process; no App restart or writer interruption
  occurred.
- Repository licensing remains a product-owner decision; visibility was not changed.

## Next action

1. No product work is pending. When all Codex writers are naturally idle, the
   existing `scripts/lattice-safe-mcp-update.py` may rotate the persistent MCP
   pointer to the already verified build-cache artifact; this is operational
   cleanup, not a TASK-106 acceptance dependency.

## Restart context

- Current product branch: `product/lattice-control-mvp`.
- Relevant plan: `PLANS.md`; complete evidence: TASK-106 workflow ledger.
- First command or file to inspect: call zero-argument `lattice_runtime_status`,
  then read `docs/reviews/WORKFLOW_LEDGER_TASK_106_2026-08-25.md` if exact
  historical evidence is needed.
