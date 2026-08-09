---
ticket_id: TASK-027
spec_id: SPEC-002
module_id: scope-check
constitution_version: 1.1
status: complete
parallel_safe: false
depends_on:
  - TASK-026
allowed_paths:
  - Cargo.toml
  - Cargo.lock
  - crates/lattice-scope-check/**
  - docs/modules/scope-check/MODULE_CONSTITUTION.md
  - docs/tickets/TASK-027-scope-check-1-1.md
  - PLANS.md
likely_files:
  - crates/lattice-scope-check/Cargo.toml
  - crates/lattice-scope-check/src/lib.rs
  - crates/lattice-scope-check/tests/scope_check.rs
branch: feature/task-026-027-workspace-scope
---

## Objective

After TASK-026 freezes normalized Git evidence, deliver Scope Check 1.1 as a
pure language-neutral detector with both-side rename checks, forbidden
precedence, fail-closed path/link handling, stable manifests/hashes, and an
explicit detection-only pass.

## Acceptance Criteria

- [x] Completes the Scope Check portion of SPEC-002 v27 AC-06 over TASK-026
  evidence without filesystem, Git, process, database, clock, or network I/O.
- [x] Exact/prefix/glob rules and allowed operations are versioned;
  forbidden matches override allowed matches.
- [x] Both rename sides are checked and malformed/absolute/traversal/empty/
  `.git`/escaped paths plus symlink/Git-link/reparse/directory/unknown kinds
  fail closed.
- [x] Rule, evidence, violation ordering, and report hashes are stable under
  input permutation and include frozen fixtures.
- [x] `PASS` is always `DETECTION_ONLY` and grants no containment, policy,
  lease, merge, or write authority.

## Non-Goals

Filesystem inspection, sandboxing, prevention, repair, cleanup, staging,
approval, Policy decisions, task transitions, persistence, orchestration,
merge readiness, user-repository tests, push, merge, deployment, or release.

## Module And Constitution Constraints

Use Scope Check 1.1. It may consume only Workspace Git's normalized evidence
and canonical byte/hash mechanics. The implementation must remain pure and
must not add caller-owned containment/currentness Booleans.

## Dependencies And Overlap

Blocked until TASK-026 is complete and its normalized evidence types are
frozen. TASK-027 owns only `crates/lattice-scope-check/**` and its own
constitution/ticket/Step-7 status update. Root Cargo files are sequential
registration surfaces; no Workspace Git implementation path is shared or
modified. Both tickets stay in one branch/worktree because this is one ordered
dependency line, never parallel work.

## TDD Behaviors

1. Validate exact/prefix/glob rules and stable allowed-operation semantics.
2. Deny forbidden overrides, out-of-scope paths, and disallowed operations.
3. Check both sides of rename and reject link/reparse/Git-link/unknown kinds.
4. Fail closed on malformed, absolute, traversal, empty, `.git`, backslash, or
   otherwise escaped path evidence, including portable trailing-dot/space and
   Windows reserved-device aliases.
5. Stable-sort all rules/evidence/violations and freeze rule/evidence/report
   hashes across input permutations without mutating inputs.
6. Return a passing report only with the explicit `DETECTION_ONLY` label.

## Verification

| Check | Command or service | Expected evidence |
|---|---|---|
| Focused unit tests | `cargo test -p lattice-scope-check --all-targets --locked` | exit 0 |
| Strict lint | `cargo clippy -p lattice-scope-check --all-targets --all-features --locked -- -D warnings` | exit 0 |
| Dependency/read-only scan | source and `cargo tree -p lattice-scope-check --depth 1` | only approved pure dependencies; no I/O |
| Node characterization | `npm.cmd run verify` | exit 0 |
| Full workspace | `cargo test --workspace --all-targets --all-features --locked` | exit 0 |

## Human Gate

none; the V2 amendment and bounded local implementation were already approved.
