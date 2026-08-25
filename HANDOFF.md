# HANDOFF

## Status

`DONE`

## Objective and scope

- Objective: 讓進行中的父工作可耐久 `BLOCKED` 於一個有界依賴，安全建立
  子 branch/worktree，整合後解除阻塞，並由 PostgreSQL 在新程序還原完整
  dependency binding 與 next action。
- Scope: Foreman/MCP/Git guard/CLI、PostgreSQL replay、delivery receipt 與
  no-restart fresh Runtime verification；不含公開網路、帳戶或安全變更。

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

- Foreman／Runtime／Ports／PostgreSQL Store、bounded CLI 與 tests，以及對應
  spec、ticket、module contracts 和 workflow ledger。逐檔範圍由 Git 保存。

## Workflow ledger

| Stage | Status | Evidence / artifact |
|---|---|---|
| Local implementation/tests | DONE | 三個已驗證功能提交；Node/Rust suites |
| Independent review | DONE | final code/architecture GO; no P0-P3 |
| Integration/CI/merge | DONE | PR #23, CI `verify` PASS, product merge |
| Install/reload | DONE | versioned artifact、Control receipt、fresh Codex reload |
| Durable replay | DONE | feature 與 post-deploy live runs |

## Verification

- Commands and exit codes: `npm.cmd run verify` 0; required Cargo test suites 0;
  `cargo fmt --check` 0; `git diff --check` 0; scoped strict Clippy 0.
- Tests/build/lint: Control 17/17; Node 117 pass/0 fail/1 platform skip;
  Runtime library 134 pass/0 fail/2 coordinated-live ignored; MCP 37/37;
  Foreman 16/16; Ports/Store/Orchestrator full suites PASS. Full Runtime strict
  Clippy retains the same 22 pre-existing diagnostics on product and feature,
  with zero TASK-106 symbol lines.
- CI: GitHub `verify` PASS in 46 seconds on PR #23 的已驗證 head。
- Runtime: installed merge artifact exposed exactly seven tools and Runtime
  projection 1.1; generation 3 replay was `VERIFIED` with `CONTINUE`.

## Review and integration

- Code review: independent final GO, P0-P3 = 0.
- Architecture review: independent final GO; PostgreSQL remains sole durable truth,
  seven tools and existing dependency directions preserved.
- Branch/worktree synchronization: isolated integration clean; product worktree,
  remote default branch and merge commit matched before this evidence-only
  finalization.
- Merge status and authorization: implementation PR #23 merged under explicit
  user authorization. No branch protection/ruleset/required review exists, so no
  service enforcement is claimed.

## Risks and open decisions

- Existing desktop MCP processes keep their open binary. Active writer locks
  prevented persistent pointer rotation; fresh reload/MCP verified the artifact
  without App restart or writer interruption.
- Repository licensing remains a product-owner decision; visibility was unchanged.

## Next action

1. No product work is pending. When all writers are naturally idle,
   `scripts/lattice-safe-mcp-update.py` may rotate the persistent MCP pointer;
   this is optional operational cleanup.

## Restart context

- Current product branch: `product/lattice-control-mvp`.
- Runtime 工作真相：LATTICE／PostgreSQL。
- 程式交付真相：GitHub 提交、PR 與 CI。
- Relevant plan: `PLANS.md`; complete evidence: TASK-106 workflow ledger.
- First command or file to inspect: call zero-argument `lattice_runtime_status`,
  then read `docs/reviews/WORKFLOW_LEDGER_TASK_106_2026-08-25.md` if exact
  historical evidence is needed.
