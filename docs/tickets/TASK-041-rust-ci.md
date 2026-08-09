---
ticket_id: TASK-041
spec_id: SPEC-002
spec_version: 27
module_id: lattice-core-bootstrap
constitution_version: 1.0
status: needs-review
parallel_safe: true
depends_on: []
allowed_paths:
  - .github/workflows/ci.yml
  - test/ci-workflow.test.js
  - docs/tickets/TASK-041-rust-ci.md
branch: feature/task-041-rust-ci
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

Status: `NEEDS_REVIEW`. The CI contract and its regression test are complete,
but integration is blocked because the exact strict Clippy command exposes 11
pre-existing errors in `lattice-hermes-adapter`. The command must not be
weakened, and the Hermes/TASK-037 files remain outside this ticket.

### Workflow Ledger

| Stage | Status | Evidence | Gate strength |
|---|---|---|---|
| Repository inspection | valid | clean base `845328dcc06d51c7554c93a09739a27ddd827941`; active TASK-038 paths inventoried | direct local evidence |
| Specification and module | valid | SPEC-002 v27 and `lattice-core-bootstrap` 1.0 read; no product contract changed | documented plus inspected |
| Ticket and worktree | valid | isolated `feature/task-041-rust-ci`; exactly three allowed paths | direct local evidence |
| TDD implementation | valid | focused test failed against the old `push: main` workflow, then passed after the CI change | machine-enforced locally |
| Focused verification | valid | CI regression test and YAML parse pass | machine-enforced locally |
| Full available verification | blocked | Node, format, and full Rust tests pass; strict Clippy exits 1 on 11 existing Hermes findings | machine-enforced locally |
| Independent code review | valid | two P2 regression-test gaps fixed and re-reviewed; one P1 baseline blocker remains | independent read-only review |
| Architecture review | skipped | workflow, test, and ticket only; no module/API/data/dependency trigger | documented-only |
| Integration and CI | blocked | no file overlap with active TASK-038, but Clippy is red; no remote Actions or branch-policy evidence | unverified remotely |

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

### Review, Integration, And Handoff

- Independent review: P0 0, P1 1, P2 0, P3 0 after both review fixes.
- Target: local `feature/task-037-full-chain-integration` at
  `845328dcc06d51c7554c93a09739a27ddd827941`; feature began at the same commit.
- Scope overlap with the active TASK-038 worktree: 0 files.
- Merge decision: `BLOCKED`; no push, merge, deployment, release, remote Actions
  run, required-check proof, or branch-protection proof.
- Root `PLANS.md` and `HANDOFF.md` are intentionally untouched because the
  active TASK-038 window currently owns both files; this ticket is the durable
  bounded handoff for TASK-041.

### Exact Next Action

After the Hermes owner makes the target branch strict-Clippy clean, synchronize
this branch with that target, rerun every command above, then push only with
explicit authorization and inspect the real GitHub Actions result before
changing this ticket to complete or declaring the check required.
