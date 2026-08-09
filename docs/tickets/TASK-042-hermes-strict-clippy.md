---
ticket_id: TASK-042
spec_id: SPEC-002
spec_version: 27
module_id: hermes-adapter
constitution_version: 1.0
status: completed
parallel_safe: true
allowed_paths:
  - crates/lattice-hermes-adapter/src/broker.rs
  - crates/lattice-hermes-adapter/src/production.rs
  - crates/lattice-hermes-adapter/src/lib.rs
  - crates/lattice-hermes-adapter/tests/**
  - docs/tickets/TASK-042-hermes-strict-clippy.md
---

# TASK-042: Hermes Strict-Clippy Baseline Cleanup

## Status

`DONE` on `feature/task-042-hermes-strict-clippy`, based exactly on
`845328dcc06d51c7554c93a09739a27ddd827941`.

## Objective

Remove the eleven Rust 1.97.1 strict-Clippy findings from
`lattice-hermes-adapter` without changing behavior, public contracts, error
classification, safety boundaries, or the production chain.

## Allowed Paths

- `crates/lattice-hermes-adapter/src/broker.rs`
- `crates/lattice-hermes-adapter/src/production.rs`
- `crates/lattice-hermes-adapter/src/lib.rs`
- `crates/lattice-hermes-adapter/tests/**` only for necessary regressions
- `docs/tickets/TASK-042-hermes-strict-clippy.md`

## Explicit Exclusions

- No lint allow attributes, lint-level changes, or weakening of `-D warnings`.
- No public-contract, error-classification, containment, credential, lease,
  thread, shell, SQL, path-selection, or production-chain change.
- No edit to the TASK-037 verifier, `PLANS.md`, `HANDOFF.md`, another crate, or
  another worktree.
- No production verifier run before the TASK-038-owned verifier checkpoint is
  available; if needed, record that as an integration gate only.
- No push, merge, deployment, or release.

## Characterization Baseline

- `cargo +1.97.1 test -p lattice-hermes-adapter --all-targets --all-features --locked`
  passed: 65 passed, 7 ignored, 0 failed.
- `cargo +1.97.1 clippy -p lattice-hermes-adapter --all-targets --all-features --locked -- -D warnings`
  failed with exactly 11 findings:
  - `too_many_lines`: 6
  - `large_stack_arrays`: 1
  - `needless_pass_by_value`: 2
  - `match_same_arms`: 1
  - `unnecessary_semicolon`: 1

## Implementation Rules

1. Preserve trace order, fixed failure codes, canonical identity checks,
   deadline/reconciliation behavior, and fail-closed branches.
2. Add a minimal behavioral regression before any helper extraction that could
   alter control flow and is not already directly characterized.
3. Fix one lint class or tightly coupled function at a time and run focused
   format, Clippy, and relevant tests after each step.
4. Finish with the requested Hermes and workspace verification matrix, an
   independent read-only code review, and one clean checkpoint commit.

## Completion Evidence

- No lint allow or lint-level change was added. All six `too_many_lines`
  findings were removed with private helpers or fixed data constants; the
  remaining five findings were fixed directly without changing public APIs.
- One minimal characterization locks completed-SSE usage behavior: unsigned
  token counts remain accepted and a negative count fails closed with
  `HERMES_EVENT_MALFORMED`.
- All Rust commands below used `CARGO_BUILD_JOBS=4` and
  `RUST_TEST_THREADS=4`:
  - `cargo +1.97.1 fmt --all -- --check`: passed.
  - `cargo +1.97.1 clippy -p lattice-hermes-adapter --all-targets --all-features --locked -- -D warnings`:
    passed with zero warnings.
  - `cargo +1.97.1 test -p lattice-hermes-adapter --all-targets --all-features --locked`:
    passed; 66 passed, 7 ignored, 0 failed.
  - `cargo +1.97.1 clippy --workspace --all-targets --all-features --locked -- -D warnings`:
    TASK-042/Hermes passed, then the workspace failed on the pre-existing,
    out-of-scope `clippy::manual_inspect` at
    `apps/lattice-runtime/src/composition.rs:2720`. TASK-042 did not modify it.
  - `cargo +1.97.1 test --workspace --all-targets --all-features --locked`:
    passed with exit code 0. No ignored/live PostgreSQL or WSL harness was run.
- `npm.cmd run verify`: passed; project check `check=ok`, 44 Node tests passed.
- `git diff --check`: passed; changed paths are limited to this ticket's
  allowlist.
- The TASK-037 production verifier was not run. Any production-sensitive
  acceptance remains an integration gate after the TASK-038-owned verifier
  checkpoint, per task scope.
- Independent read-only code review reported `No findings` with P0-P3 all
  zero. The reviewer independently passed one-turn 5/5, SSE usage 1/1, and
  production-provider 11/11 focused tests and found no architecture-review
  trigger. The checkpoint has no TASK-042 review blocker.
