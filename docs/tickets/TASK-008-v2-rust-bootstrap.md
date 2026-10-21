---
ticket_id: TASK-008
spec_id: SPEC-002
spec_version: 2
module_id: lattice-core-bootstrap
constitution_version: 1.0
status: completed
parallel_safe: false
depends_on: []
allowed_paths:
  - Cargo.toml
  - Cargo.lock
  - .gitignore
  - crates/lattice-core/**
  - apps/lattice-cli/**
  - db/migrations/0001_bootstrap.sql
  - scripts/check-project.mjs
  - README.md
  - PLANS.md
  - HANDOFF.md
  - docs/modules/lattice-core-bootstrap/**
  - docs/modules/lattice-cli/**
  - docs/plans/V2_BOOTSTRAP_PRESERVATION.md
  - docs/tickets/TASK-008-v2-rust-bootstrap.md
  - docs/workflow/WORKFLOW_LEDGER.md
  - docs/reviews/CODE_REVIEW_TASK_008_2026-07-29.md
  - docs/reviews/ARCHITECTURE_REVIEW_TASK_008_2026-07-29.md
  - docs/reviews/INTEGRATION_TASK_008_2026-07-29.md
likely_files:
  - Cargo.toml
  - crates/lattice-core/Cargo.toml
  - crates/lattice-core/src/lib.rs
  - crates/lattice-core/tests/platform_manifest.rs
  - apps/lattice-cli/Cargo.toml
  - apps/lattice-cli/src/main.rs
  - db/migrations/0001_bootstrap.sql
  - scripts/check-project.mjs
branch: feature/v2-rust-postgres-bootstrap
---

## Objective

Create a buildable, dependency-free Rust workspace that visibly establishes
LATTICE DevOS as a general AI development platform and lists the approved
OpenClaw, Codex, Graphify, Hermes, PostgreSQL, Codebase Memory, and Guardian
lanes without starting or installing any of them.

## Acceptance Criteria

- [x] The workspace builds with the installed Rust toolchain and contains
  `lattice-core` plus the read-only `lattice-cli`.
- [x] The core manifest lists every approved component exactly once.
- [x] Graphify and Hermes are marked read-only; PostgreSQL is marked durable
  truth; Codex is marked sole writer; Guardian is marked approval-gated.
- [x] `cargo run -p lattice-cli -- status` prints the platform and component
  modes without network, database, process, Git, or credential access.
- [x] The SQL draft creates only the `control`, `memory`, and `readmodel`
  namespaces and is not executed.
- [x] The dirty V1 worktree and its Node code/test changes remain untouched.

## Non-Goals

- Implement live adapters, task execution, persistence, migrations, login,
  installation, payment, publishing, deployment, or public listeners.
- Port V1 behavior or finalize the full ADR-004 crate decomposition.

## Module And Constitution Constraints

- `lattice-core-bootstrap` 1.0 owns the inert component manifest.
- `lattice-cli` 1.0 owns read-only rendering and recovery CLI behavior.
- SPEC-002 and ADR-004 through ADR-007 remain authoritative.
- The CLI is not a second normal gateway.

## Dependencies And Overlap

This ticket is not parallel-safe because it creates the root Cargo workspace,
lockfile, and initial public identifiers. Later crate tickets depend on these
names and must not edit the same workspace manifest concurrently.

## TDD Behaviors

1. RED: a core contract test cannot compile before the component manifest
   exists; GREEN: the public manifest satisfies identity, uniqueness, and mode
   assertions.
2. RED: CLI rendering tests cannot compile before rendering exists; GREEN:
   status output is deterministic and covers every component.

## Verification

| Check | Command or service | Expected evidence |
|---|---|---|
| Core focused test | `cargo test -p lattice-core --test platform_manifest` | all manifest assertions pass |
| CLI focused test | `cargo test -p lattice-cli` | rendering and invalid-command tests pass |
| Format | `cargo fmt --check` | exit 0 |
| Lint | `cargo clippy --workspace --all-targets --all-features -- -D warnings` | exit 0 |
| Full Rust suite | `cargo test --workspace` | exit 0 |
| Smoke | `cargo run -p lattice-cli -- status` | expected inert component list |
| Repository check | `npm.cmd run check` | exit 0 |
| Diff hygiene | `git diff --check` | exit 0 |

## Human Gate

None for this inert local bootstrap. Any external install, database execution,
credential use, live adapter, model call, promotion, push, merge, publication,
deployment, or public network exposure requires a separate user decision.
