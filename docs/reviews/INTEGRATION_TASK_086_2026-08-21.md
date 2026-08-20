# Integration Evidence — TASK-086

## Inputs

| Input | Commit | Remote equality | Tree |
|---|---|---|---|
| TASK-041 | `e3b10b42a88fb7484fef1d4dc668b1ebdd40e9a0` | `origin/feature/task-041-rust-ci`: equal (0/0) | clean |
| TASK-042 | `a41dc7c3d9d6440cc4df66007c92ce9eb30c8953` | `origin/feature/task-042-hermes-strict-clippy`: equal (0/0) | clean |

`TASK-086` was unoccupied in local/remote refs, registered worktrees, and
tracked ticket text before this branch was created. The pre-existing
`integration/task-041-task-042` was local-only at `f4c2dbe` and historical;
it was not modified.

## Combined Verification

| Command | Result |
|---|---|
| `cargo +1.97.1 fmt --all -- --check` | pass |
| `cargo +1.97.1 clippy --workspace --all-targets --all-features --locked -- -D warnings` | fail: one pre-existing `clippy::manual_inspect` at `apps/lattice-runtime/src/composition.rs:2720`; no Hermes finding |
| `cargo +1.97.1 test --workspace --all-targets --all-features --locked` | pass, exit 0 |
| `cargo +1.97.1 test -p lattice-hermes-adapter --all-targets --all-features --locked` | pass: 66 passed, 7 ignored |
| `node --test test/ci-workflow.test.js` | pass: 1 test |
| `npm.cmd run check` | pass |
| `npm.cmd run verify` | pass, exit 0: project check plus 45 Node tests |
| `git diff --check` | pass after evidence documentation |

## Integration Classification

`BLOCKED`: the source histories merged cleanly and TASK-041's Hermes-specific
blocker is resolved, but the exact Rust-CI workspace Clippy gate remains red
for the unrelated runtime lint. No primary-branch merge, deployment, release,
or remote GitHub Actions result is claimed.
