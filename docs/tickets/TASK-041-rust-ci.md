---
ticket_id: TASK-041
spec_id: SPEC-002
spec_version: 27
module_id: lattice-core-bootstrap
constitution_version: 1.0
status: completed
parallel_safe: true
depends_on: []
allowed_paths:
  - .github/workflows/ci.yml
  - test/ci-workflow.test.js
  - docs/tickets/TASK-041-rust-ci.md
branch: feature/task-041-rust-ci
delivery_remote: origin
delivery_repository: github.com/z72124223/lattice-devos
delivery_push: authorized_non_force_feature_branch
delivery_archive: keep_open
---

# Rust GitHub Actions CI

## Objective

Turn the documented Rust verification commands into a GitHub Actions gate
without touching the TASK-037/TASK-038 runtime, MCP, PostgreSQL, or writer
paths. Preserve the existing Node verification job and make pushes to the
repository's actual feature branches trigger the workflow.

## Acceptance Criteria

- Pull requests and pushes to every branch trigger the workflow.
- The existing Node `verify` job still runs `npm ci --ignore-scripts` and
  `npm run verify` on Node 24.
- A separate Rust job installs exact Rust 1.97.1 with `clippy` and `rustfmt`.
- Rust runs the repository-documented locked gates:
  - `cargo +1.97.1 fmt --all -- --check`
  - `cargo +1.97.1 clippy --workspace --all-targets --all-features --locked -- -D warnings`
  - `cargo +1.97.1 test --workspace --all-targets --all-features --locked`
- Official GitHub actions are pinned to the currently verified immutable
  release commits, and workflow permissions remain read-only.
- A local regression test locks the trigger, action, toolchain, and command
  contract; focused and full local verification pass.

## Non-Goals

- Push, merge, deploy, release, or change GitHub branch protection.
- Claim that a local workflow file proves a remote Actions run or required
  check.
- Run the marker-owned PostgreSQL or ignored Graphify/Hermes live gates in CI.
- Modify product/runtime code or any TASK-037/TASK-038 path.

## Verification

- `node --test test/ci-workflow.test.js`
- `npm.cmd run verify`
- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`
- `cargo test --workspace --all-targets --all-features --locked`
- `git diff --check`

## Current Checkpoint

Status: `COMPLETED` by history-preserving integration evidence. The source
branch's own committed functional checkpoint remains `e08f994`; its terminal
metadata checkpoint `e3b10b4` truthfully recorded that strict Clippy was then
blocked by 11 out-of-scope Hermes findings. It does not itself contain the
Hermes or runtime repairs.

TASK-086 later integrated that exact source commit with TASK-042 at merge
`5b59bf4414889d2c674a934ccf32e9887da26883`, whose parents are exactly
`e3b10b4` and TASK-042 `a41dc7c3d9d6440cc4df66007c92ce9eb30c8953`. TASK-086
then integrated TASK-088 `68fd1412bd7cc63a0569fae9251c626de0c49de0` at merge
`93bf2a8564b04d4c03f08cebfb0ff5b6356b5397`. The delivered descendant
`94bfbab05f610aff0a0956c802a8f662accf26d0` preserves all three histories and
records passing strict Clippy, full workspace tests, focused Hermes tests,
Node check/verify, formatting, and diff checks. This changes the ticket's
integration outcome, not the historical content of this source branch; remote
GitHub Actions, required checks, branch protection, primary-branch merge,
deployment, and release remain unverified and unauthorized.

### Workflow Ledger

| Stage | Status | Evidence | Gate strength |
|---|---|---|---|
| Repository inspection | valid | clean base `845328dcc06d51c7554c93a09739a27ddd827941`; active TASK-038 paths inventoried | direct local evidence |
| Specification and module | valid | SPEC-002 v27 and `lattice-core-bootstrap` 1.0 read; no product contract changed | documented plus inspected |
| Ticket and worktree | valid | isolated `feature/task-041-rust-ci`; exactly three allowed paths | direct local evidence |
| TDD implementation | valid | focused test failed against the old `push: main` workflow, then passed after the CI change | machine-enforced locally |
| Focused verification | valid | CI regression test and YAML parse pass | machine-enforced locally |
| Source-branch revalidation | blocked historically | 2026-08-21 source rerun: Node, format, and full Rust tests pass; strict Clippy exits 1 on 11 existing Hermes findings | machine-enforced locally |
| History-preserving integration revalidation | valid | TASK-086 descendant `94bfbab` preserves TASK-041/TASK-042/TASK-088 and records the exact strict Clippy matrix passing | machine-enforced locally on the integration descendant |
| Independent code review | valid | original TASK-041 review closed its P2 gaps; TASK-086 review reports no unresolved P0-P3 after the descendant integration | independent read-only review |
| Architecture review | skipped locally | TASK-041 changes only workflow/test/ticket; TASK-086 separately records no integration architecture trigger or blocker | documented-only plus read-only integration review |
| Integration and CI | valid locally / unverified remotely | TASK-086 descendant passes the exact Rust-CI matrix; no remote Actions or branch-policy evidence | machine-enforced locally / unverified remotely |

### Verification Evidence

- `node --test test/ci-workflow.test.js`: exit 0 after the expected pre-change
  RED failure; 1 passed.
- `npm.cmd run verify`: exit 0; project check passed and 45 Node tests passed.
- YAML parse with `yaml.BaseLoader`: exit 0; jobs are `verify` and `rust`.
- `cargo +1.97.1 fmt --all -- --check`: exit 0.
- `cargo +1.97.1 test --workspace --all-targets --all-features --locked`:
  exit 0.
- `cargo +1.97.1 clippy --workspace --all-targets --all-features --locked -- -D warnings`:
  exit 1; 11 existing errors in `crates/lattice-hermes-adapter/src/{broker,production,lib}.rs`.
- `git diff --check`: exit 0.

### 2026-08-21 Terminal Revalidation

- `node --test test/ci-workflow.test.js`: exit 0; 1 test passed.
- `npm.cmd run check`: exit 0; `check=ok files=410 constitutions=24 tickets=25 current_tasks=1`.
- `cargo fmt --all -- --check`: exit 0.
- `cargo test --workspace --all-targets --all-features --locked`: exit 0.
- `npm.cmd run verify`: exit 0.
- `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`:
  exit 1; the same 11 findings remain solely in
  `crates/lattice-hermes-adapter/src/{broker,production,lib}.rs`, outside
  TASK-041's allowed paths.
- `git diff --check`: exit 0 before this terminal-metadata update.

### 2026-08-21 TASK-086 Descendant Acceptance Evidence

This is not a claim that `e3b10b4` contains the repair. Direct read-only Git
checks established that `e3b10b4`, TASK-042 `a41dc7c`, and TASK-088 `68fd141`
are all ancestors of delivered TASK-086 head `94bfbab`. TASK-086's committed
integration record reports the following at that descendant:

- `cargo +1.97.1 fmt --all -- --check`: pass.
- `cargo +1.97.1 clippy --workspace --all-targets --all-features --locked -- -D warnings`:
  pass; neither the former eleven Hermes findings nor the runtime
  `manual_inspect` finding remains.
- `cargo +1.97.1 test --workspace --all-targets --all-features --locked`:
  pass; focused `lattice-hermes-adapter` tests pass (66 passed, 7 ignored).
- `node --test test/ci-workflow.test.js`, `npm.cmd run check`, `npm.cmd run verify`,
  and `git diff --check`: pass.

The supporting committed records are
`docs/reviews/{CODE_REVIEW,ARCHITECTURE_REVIEW,INTEGRATION}_TASK_086_2026-08-21.md`
at TASK-086 `94bfbab`. They report no unresolved P0-P3 finding and no
architecture blocker, while preserving the distinct unverified remote-CI and
primary-branch gates.

### Review, Integration, And Handoff

- Independent review: the original source review recorded P0 0, P1 1, P2 0,
  P3 0; TASK-086's later integration review reports P0 0, P1 0, P2 0, P3 0
  after the strict-Clippy repairs are included in the descendant.
- Target: local `feature/task-037-full-chain-integration` at
  `845328dcc06d51c7554c93a09739a27ddd827941`; feature began at the same commit.
- Scope overlap with the active TASK-038 worktree: 0 files.
- Integration decision: `COMPLETED` for the Rust-CI contract through TASK-086's
  delivered history-preserving descendant. No remote Actions run,
  required-check proof, branch-protection proof, primary-branch merge,
  deployment, or release is claimed. This ticket remains `keep_open` for that
  separate operational boundary.
- Root `PLANS.md` and `HANDOFF.md` are intentionally untouched because the
  active TASK-038 window currently owns both files; this ticket is the durable
  bounded handoff for TASK-041.

### Exact Next Action

For remote CI or branch-protection readiness, run the GitHub Actions workflow
on an authorized integration/primary target and inspect its live result. That
is separate from the completed local history-preserving acceptance recorded
here.
